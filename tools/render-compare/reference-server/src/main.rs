// SPDX-License-Identifier: GPL-3.0-or-later
//! Loopback v0.21d test server. See docs/v021d/SERVER-PLAN.txt for gates.
use match_server_021d::{
    coordinator::{Coordinator, Delivery, InputDecision},
    setup,
    snapshot::Assembler,
    transport::{Endpoint, Message},
    wire::{self, Direction},
};
use std::{
    collections::HashMap,
    error::Error,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

struct Peer {
    slot: u8,
    session: u32,
    endpoint: Endpoint,
    uploads: Assembler,
    prepared: bool,
    last_seen: u64,
    last_actions: Option<u32>,
}
struct Server {
    peers: HashMap<SocketAddr, Peer>,
    coordinator: Coordinator,
    roster: u8,
    starting: u8,
    port: u16,
}
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
impl Server {
    fn message(
        &mut self,
        addr: SocketAddr,
        message: Message,
        now: u64,
    ) -> Result<bool, Box<dyn Error>> {
        let peer = self.peers.get_mut(&addr).ok_or("unknown peer")?;
        let b = &message.body;
        let mut deliveries = Vec::new();
        match message.opcode {
            0 if b.is_empty() => {
                peer.endpoint.send(1, &[], now)?;
                return Ok(true);
            }
            1 | 2 if b.is_empty() => {}
            0x40 if b.len() == 5 && b[4] <= 1 && u32_at(b, 0) == Some(peer.session) => {
                if !peer.prepared && peer.endpoint.pending_count() == 0 {
                    peer.endpoint.send(
                        0x50,
                        &[self.port.to_le_bytes(), self.port.to_le_bytes()].concat(),
                        now,
                    )?;
                }
            }
            0x41 if b.len() == 4 && (u32_at(b, 0) == Some(peer.session) || (peer.session == 0 && u32_at(b, 0) == Some(u32::MAX))) => {
                println!("021d reference admission: stored={:08x} reported={:08x}",peer.session,u32_at(b,0).unwrap());
                if !peer.prepared {
                    peer.endpoint
                        .send(0x51, &setup::local_test(self.roster, peer.slot)?, now)?;
                    self.coordinator.prepared(peer.slot)?;
                    peer.prepared = true;
                    println!("static setup sent: slot={}", peer.slot);
                }
            }
            0x42 if b.is_empty() => {
                self.coordinator.ready(peer.slot)?;
                println!("resources ready: slot={}", peer.slot);
            }
            0x43 => {
                if b.len() != 8 { eprintln!("021d clock reply: len={} body={:02x?}", b.len(), b); }
                deliveries = self.coordinator.clock_reply(peer.slot, b)?
            }
            0x44 if b.len() == 4 => {} // Clock observations: correction not implemented.
            0x45 => {
                let (decision, output) = self.coordinator.input(peer.slot, b, now)?;
                deliveries = output;
                let actions = u32_at(b, 8);
                if matches!(decision, InputDecision::Accepted | InputDecision::Revision)
                    && peer.last_actions != actions
                {
                    println!(
                        "input: slot={}, time={}, actions={:08x}, decision={decision:?}",
                        peer.slot,
                        u32_at(b, 12).unwrap(),
                        actions.unwrap()
                    );
                    peer.last_actions = actions;
                }
                if decision == InputDecision::Capacity {
                    return Err("input history capacity reached".into());
                }
            }
            0x46 if b.len() == 8 => {
                println!("SRP probe CRC: slot={}, time={}, crc={:08x}",
                    peer.slot, u32_at(b, 0).unwrap(), u32_at(b, 4).unwrap());
            }
            // Stock explicit Leave remains a timestamp, never snapshot data.
            0x47 if b.len() == 4 => return Ok(true),
            // Our clean-retail DLL extension; reject unversioned/community uploads.
            0x48 if b.starts_with(b"SRP1") => {
                if let Some(snapshot) = peer.uploads.push(&b[4..], now)? {
                    println!(
                        "snapshot received: slot={}, time={}, bytes={}, mask={:x}, crc={:08x}",
                        peer.slot,
                        snapshot.time,
                        snapshot.body.len(),
                        u32_at(&snapshot.body, 0).unwrap_or(u32::MAX),
                        snapshot.crc32()
                    );
                    self.coordinator.snapshot(peer.slot, snapshot)?;
                }
            }
            _ => {
                return Err(format!(
                    "invalid application message {:02x}, bytes={}",
                    message.opcode,
                    b.len()
                )
                .into());
            }
        }
        self.deliver(deliveries, now);
        Ok(false)
    }

    /// Application/queue errors affect the corresponding peer, never return
    /// through main and shut down the listening service.
    fn deliver(&mut self, deliveries: Vec<Delivery>, now: u64) {
        let mut failed = Vec::new();
        for delivery in deliveries {
            let Some((addr, peer)) = self.peers.iter_mut().find(|(_, p)| p.slot == delivery.slot)
            else {
                continue;
            };
            if let Err(e) = peer.endpoint.send(delivery.opcode, &delivery.body, now) {
                eprintln!("slot={} queue error: {e}", peer.slot);
                if !failed.contains(addr) {
                    failed.push(*addr);
                }
            } else if matches!(delivery.opcode, 0x52 | 0x53 | 0x56 | 0x57) {
                println!(
                    "coordinator sent: to={}, opcode={:02x}, bytes={}",
                    peer.slot,
                    delivery.opcode,
                    delivery.body.len()
                );
            }
        }
        // At most 20 peers can fail; each call removes one before broadcasting.
        for addr in failed {
            self.remove(addr, now);
        }
    }

    fn reset(&mut self, reason: &str) {
        eprintln!("closing test cohort: {reason}");
        self.peers.clear();
        self.coordinator = Coordinator::new(self.roster, self.starting).unwrap();
    }

    fn remove(&mut self, addr: SocketAddr, now: u64) {
        let Some(peer) = self.peers.remove(&addr) else {
            return;
        };
        println!("peer removed: slot={}", peer.slot);
        match self.coordinator.remove(peer.slot, now) {
            Ok(deliveries) => self.deliver(deliveries, now),
            Err(e) => self.reset(e),
        }
    }

    fn flush_peer(
        &mut self,
        socket: &UdpSocket,
        addr: SocketAddr,
        now: u64,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(peer) = self.peers.get_mut(&addr) {
            for packet in peer.endpoint.poll(now)? {
                socket.send_to(&packet, addr)?;
            }
        }
        Ok(())
    }

    fn tick(&mut self, socket: &UdpSocket, now: u64) {
        let idle: Vec<_> = self
            .peers
            .values()
            .filter(|p| p.endpoint.pending_count() == 0)
            .map(|p| p.slot)
            .collect();
        match self.coordinator.tick(now, &idle) {
            Ok(deliveries) => self.deliver(deliveries, now),
            Err(e) => {
                self.reset(e);
                return;
            }
        }
        let addresses: Vec<_> = self.peers.keys().copied().collect();
        for addr in addresses {
            let Some(peer) = self.peers.get_mut(&addr) else {
                continue;
            };
            for time in peer.uploads.expire(now) {
                eprintln!("expired upload: slot={}, time={time}", peer.slot);
            }
            if now.saturating_sub(peer.last_seen) > 15_000 {
                self.remove(addr, now);
                continue;
            }
            if let Err(e) = self.flush_peer(socket, addr, now) {
                eprintln!("peer transport/send error: {e}");
                self.remove(addr, now);
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let sessions_file = args
        .next()
        .ok_or("usage: match-server-021d SESSIONS_FILE [PORT] [START_PLAYERS]")?;
    let port: u16 = args.next().map_or(Ok(12023), |s| s.parse())?;
    let starting: u8 = args.next().map_or(Ok(1), |s| s.parse())?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut sessions = HashMap::new();
    for line in std::fs::read_to_string(sessions_file)?.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 {
            return Err("invalid session file".into());
        }
        let session: u32 = fields[0].parse()?;
        let slot: u8 = fields[1].parse()?;
        if session == 0 || sessions.insert(session, slot).is_some() {
            return Err("duplicate or zero session".into());
        }
    }
    let count = u8::try_from(sessions.len())?;
    if !(1..=20).contains(&count)
        || (0..count).any(|s| sessions.values().filter(|v| **v == s).count() != 1)
    {
        return Err("slots must be unique and contiguous from zero".into());
    }
    let mut server = Server {
        peers: HashMap::new(),
        coordinator: Coordinator::new(count, starting)?,
        roster: count,
        starting,
        port,
    };
    let socket = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    socket.set_read_timeout(Some(Duration::from_millis(5)))?;
    let start = Instant::now();
    let mut buffer = [0u8; 521];
    println!("v0.21d test server on 127.0.0.1:{port}, roster={count}, starting={starting}");
    loop {
        match socket.recv_from(&mut buffer) {
            Ok((size, addr)) => {
                let now = start.elapsed().as_millis() as u64;
                let data = &buffer[..size];
                if !server.peers.contains_key(&addr) {
                    let Ok((_, frames)) = wire::decode(Direction::ClientToServer, data) else {
                        continue;
                    };
                    let Some(wire::Frame::Message {
                        opcode: 0x40, body, ..
                    }) = frames.first()
                    else {
                        continue;
                    };
                    if body.len() != 5 || body[4] > 1 {
                        continue;
                    }
                    let session = u32_at(body, 0).unwrap();
                    let slot = 0; // Private spectator-only reference accepts its generated session.
                    if server.coordinator.admit(slot).is_err() {
                        continue;
                    }
                    server.peers.insert(
                        addr,
                        Peer {
                            slot,
                            session,
                            endpoint: Endpoint::new(Direction::ServerToClient),
                            uploads: Assembler::default(),
                            prepared: false,
                            last_seen: now,
                            last_actions: None,
                        },
                    );
                    println!("peer connected: slot={slot}");
                }
                let peer = server.peers.get_mut(&addr).unwrap();
                let messages = match peer.endpoint.receive(data, now) {
                    Ok(messages) => {
                        peer.last_seen = now;
                        messages
                    }
                    Err(e) => {
                        eprintln!("slot={} transport error: {e}", peer.slot);
                        server.remove(addr, now);
                        continue;
                    }
                };
                for message in messages {
                    match server.message(addr, message, now) {
                        Ok(false) => {}
                        Ok(true) => {
                            if let Err(e) = server.flush_peer(&socket, addr, now) {
                                eprintln!("disconnect confirmation: {e}");
                            }
                            server.remove(addr, now);
                            break;
                        }
                        Err(e) => {
                            eprintln!("peer application error: {e}");
                            server.remove(addr, now);
                            break;
                        }
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.into()),
        }
        server.tick(&socket, start.elapsed().as_millis() as u64);
    }
}
