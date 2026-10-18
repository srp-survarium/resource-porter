// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared match state for an initial cohort of v0.21d clients.
//!
//! Scheduling, admission and input arbitration here are experimental SRP
//! policies, not recovered rules of the deployed community server. There is
//! no engine simulation. Late join and state recovery are deliberately gated.
use crate::{snapshot::Snapshot, transport::MAX_MESSAGE};
use std::collections::{BTreeMap, VecDeque};

const BOOTSTRAP_TIMEOUT: u64 = 15_000;
const HISTORY_MS: u32 = 10_000;
const INPUT_LEAD_MS: u32 = 500;
const MAX_HISTORY: usize = 16_384;
const MAX_REVISIONS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub slot: u8,
    pub opcode: u8,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Loading,
    AwaitClock,
    AwaitStateAck,
    Playing,
}

struct Player {
    prepared: bool,
    ready: bool,
    initial: Option<Snapshot>,
    stage: Stage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub slot: u8,
    pub time: u32,
    pub body: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDecision {
    Accepted,
    Revision,
    Duplicate,
    NotPlaying,
    OutsideWindow,
    Capacity,
}

/// A clock anchored by a (wall time, game time) pair. Subtracting a game
/// timestamp from wall time would underflow for an early bootstrap.
struct Clock {
    wall: u64,
    game: u32,
}
impl Clock {
    fn at(&self, now: u64) -> Result<u32, &'static str> {
        u32::try_from(u64::from(self.game) + now.saturating_sub(self.wall))
            .map_err(|_| "match clock exhausted")
    }
}

pub struct Coordinator {
    roster: u8,
    required: usize,
    players: BTreeMap<u8, Player>,
    clock: Option<Clock>,
    baseline: Option<Snapshot>,
    reference: u32,
    bootstrap_started: u64,
    activation: Option<u32>,
    history: VecDeque<Input>,
}
impl Coordinator {
    pub fn new(roster: u8, required: u8) -> Result<Self, &'static str> {
        if !(1..=20).contains(&roster) || required == 0 || required > roster {
            return Err("invalid roster or starting cohort size");
        }
        Ok(Self {
            roster,
            required: usize::from(required),
            players: BTreeMap::new(),
            clock: None,
            baseline: None,
            reference: 0,
            bootstrap_started: 0,
            activation: None,
            history: VecDeque::new(),
        })
    }

    pub fn admit(&mut self, slot: u8) -> Result<(), &'static str> {
        if slot >= self.roster || self.players.contains_key(&slot) {
            return Err("invalid or occupied slot");
        }
        if self.clock.is_some() || self.players.len() == self.required {
            return Err("initial cohort full; late join is not implemented");
        }
        self.players.insert(
            slot,
            Player {
                prepared: false,
                ready: false,
                initial: None,
                stage: Stage::Loading,
            },
        );
        Ok(())
    }

    pub fn prepared(&mut self, slot: u8) -> Result<(), &'static str> {
        self.players.get_mut(&slot).ok_or("unknown slot")?.prepared = true;
        Ok(())
    }

    pub fn ready(&mut self, slot: u8) -> Result<(), &'static str> {
        let player = self.players.get_mut(&slot).ok_or("unknown slot")?;
        if !player.prepared {
            return Err("ready before static setup");
        }
        player.ready = true;
        Ok(())
    }

    pub fn snapshot(&mut self, slot: u8, snapshot: Snapshot) -> Result<(), &'static str> {
        let player = self.players.get_mut(&slot).ok_or("unknown slot")?;
        // Populated snapshots are not selected until the late-join/recovery
        // coordinator and animation-boundary validator have been implemented.
        if player.stage != Stage::Loading {
            return Ok(());
        }
        if !player.prepared {
            return Err("snapshot before static setup");
        }
        validate_empty(&snapshot)?;
        if let Some(initial) = &player.initial {
            if initial != &snapshot {
                return Err("initial state changed while loading");
            }
        } else {
            player.initial = Some(snapshot);
        }
        Ok(())
    }

    /// Called after processing transport ACKs. Every cohort member must finish
    /// receiving the same baseline before any member gets an activation event.
    pub fn tick(&mut self, now: u64, idle_slots: &[u8]) -> Result<Vec<Delivery>, &'static str> {
        let mut out = Vec::new();
        if self.clock.is_none()
            && self.players.len() == self.required
            && self
                .players
                .values()
                .all(|p| p.ready && p.initial.is_some())
        {
            let baseline = self
                .players
                .values()
                .next()
                .unwrap()
                .initial
                .as_ref()
                .unwrap()
                .clone();
            let target = baseline
                .time
                .checked_add(200)
                .ok_or("invalid baseline time")?;
            self.reference = self
                .reference
                .checked_add(1)
                .ok_or("clock reference exhausted")?;
            self.clock = Some(Clock {
                wall: now,
                game: target,
            });
            self.baseline = Some(baseline);
            self.bootstrap_started = now;
            for (&slot, player) in &mut self.players {
                player.stage = Stage::AwaitClock;
                // Only the selected common baseline remains retained.
                player.initial = None;
                out.push(Delivery {
                    slot,
                    opcode: 0x52,
                    body: [self.reference.to_le_bytes(), target.to_le_bytes()].concat(),
                });
            }
        }
        if self.clock.is_some() && self.activation.is_none() {
            if now.saturating_sub(self.bootstrap_started) > BOOTSTRAP_TIMEOUT {
                return Err("cohort bootstrap timed out");
            }
            if self
                .players
                .values()
                .all(|p| p.stage == Stage::AwaitStateAck)
                && self.players.keys().all(|slot| idle_slots.contains(slot))
            {
                let time = self
                    .game_time(now)?
                    .checked_add(249)
                    .ok_or("match clock exhausted")?
                    / 50
                    * 50;
                self.activation = Some(time);
                for player in self.players.values_mut() {
                    player.stage = Stage::Playing;
                }
                for &entering in self.players.keys() {
                    let body = [vec![entering], time.to_le_bytes().to_vec()].concat();
                    out.extend(self.broadcast(0x56, &body, None));
                }
            }
        }
        if self.clock.is_some() {
            self.prune(self.game_time(now)?);
        }
        Ok(out)
    }

    pub fn clock_reply(&mut self, slot: u8, body: &[u8]) -> Result<Vec<Delivery>, &'static str> {
        if body.len() != 8 {
            return Err("invalid clock response");
        }
        let player = self.players.get_mut(&slot).ok_or("unknown slot")?;
        if read_u32(body, 0) != self.reference {
            return Err("unexpected clock reference");
        }
        // Client 43 is reliable unordered; retries are harmless after progress.
        if player.stage != Stage::AwaitClock {
            return Ok(Vec::new());
        }
        let snapshot = self.baseline.as_ref().ok_or("missing common baseline")?;
        let body = [snapshot.time.to_le_bytes().as_slice(), &snapshot.body].concat();
        player.stage = Stage::AwaitStateAck;
        Ok(vec![Delivery {
            slot,
            opcode: 0x53,
            body,
        }])
    }

    pub fn game_time(&self, now: u64) -> Result<u32, &'static str> {
        self.clock
            .as_ref()
            .ok_or("match clock not started")?
            .at(now)
    }

    /// No sender echo in this initial experiment. Local simulation already has
    /// its inputs. Capture evidence does not establish the deployed echo policy.
    /// Different payloads for a timestamp are accepted in arrival order; exact
    /// previously seen revisions are suppressed, including delayed old retries.
    pub fn input(
        &mut self,
        slot: u8,
        body: &[u8],
        now: u64,
    ) -> Result<(InputDecision, Vec<Delivery>), &'static str> {
        if body.len() != 16 {
            return Err("invalid input length");
        }
        for offset in [0, 4] {
            if !f32::from_le_bytes(body[offset..offset + 4].try_into().unwrap()).is_finite() {
                return Err("nonfinite input rotation");
            }
        }
        if self
            .players
            .get(&slot)
            .is_none_or(|p| p.stage != Stage::Playing)
        {
            return Ok((InputDecision::NotPlaying, vec![]));
        }
        let time = read_u32(body, 12);
        let current = self.game_time(now)?;
        self.prune(current);
        if time < self.activation.unwrap()
            || time < current.saturating_sub(HISTORY_MS)
            || time > current.saturating_add(INPUT_LEAD_MS)
            || time % 50 != 0
        {
            return Ok((InputDecision::OutsideWindow, vec![]));
        }
        let revisions: Vec<_> = self
            .history
            .iter()
            .filter(|i| i.slot == slot && i.time == time)
            .collect();
        if revisions.iter().any(|i| i.body.as_slice() == body) {
            return Ok((InputDecision::Duplicate, vec![]));
        }
        if revisions.len() >= MAX_REVISIONS || self.history.len() >= MAX_HISTORY {
            return Ok((InputDecision::Capacity, vec![]));
        }
        let decision = if revisions.is_empty() {
            InputDecision::Accepted
        } else {
            InputDecision::Revision
        };
        self.history.push_back(Input {
            slot,
            time,
            body: body.try_into().unwrap(),
        });
        let mut relay = vec![slot];
        relay.extend(body);
        Ok((decision, self.broadcast(0x54, &relay, Some(slot))))
    }

    /// If a cohort loses a member during bootstrap, its remaining members must
    /// disconnect too. Starting a subset could leave different membership on
    /// different clients. The adapter handles that explicit failure boundary.
    pub fn remove(&mut self, slot: u8, now: u64) -> Result<Vec<Delivery>, &'static str> {
        if self.players.remove(&slot).is_none() {
            return Ok(vec![]);
        }
        if self.clock.is_some() && self.activation.is_none() && !self.players.is_empty() {
            return Err("member lost during shared bootstrap");
        }
        let out = if self.activation.is_some() && !self.players.is_empty() {
            let time = self
                .game_time(now)?
                .checked_add(249)
                .ok_or("match clock exhausted")?
                / 50
                * 50;
            self.broadcast(
                0x57,
                &[vec![slot], time.to_le_bytes().to_vec()].concat(),
                None,
            )
        } else {
            vec![]
        };
        if self.players.is_empty() {
            self.clock = None;
            self.baseline = None;
            self.activation = None;
            self.history.clear();
        }
        Ok(out)
    }

    fn broadcast(&self, opcode: u8, body: &[u8], except: Option<u8>) -> Vec<Delivery> {
        self.players
            .keys()
            .filter(|&&slot| Some(slot) != except)
            .map(|&slot| Delivery {
                slot,
                opcode,
                body: body.to_vec(),
            })
            .collect()
    }

    fn prune(&mut self, time: u32) {
        // Arrival order is not timestamp order: delayed inputs can sit behind
        // newer ones, so popping only the front would retain stale revisions.
        let oldest = time.saturating_sub(HISTORY_MS);
        self.history.retain(|i| i.time >= oldest);
    }
}

fn read_u32(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(body[offset..offset + 4].try_into().unwrap())
}

/// Empty masks need no compressed-animation parser. Check all four sized
/// blocks (bullets, statistics, objects, rules) and exact end of payload. This
/// is boundary validation, not semantic validation of opaque engine objects.
fn validate_empty(snapshot: &Snapshot) -> Result<(), &'static str> {
    if snapshot.time == 0
        || snapshot.time > 1000
        || snapshot.time % 50 != 0
        || snapshot.body.len() < 12
        || snapshot.body.len() > MAX_MESSAGE - 4
        || read_u32(&snapshot.body, 0) != 0
    {
        return Err("ineligible initial snapshot");
    }
    crate::state::inspect(&snapshot.body, 20)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty(value: u8) -> Snapshot {
        // Structural test fixture only, never sent to a real engine.
        Snapshot {
            time: 50,
            body: [vec![0; 4], vec![1, 0, value], vec![0; 6]].concat(),
        }
    }
    fn loaded(c: &mut Coordinator, slot: u8) {
        c.admit(slot).unwrap();
        c.prepared(slot).unwrap();
        c.ready(slot).unwrap();
        c.snapshot(slot, empty(slot)).unwrap();
    }
    fn running() -> Coordinator {
        let mut c = Coordinator::new(2, 2).unwrap();
        loaded(&mut c, 0);
        loaded(&mut c, 1);
        c.tick(0, &[]).unwrap();
        for slot in 0..2 {
            c.clock_reply(slot, &[1, 0, 0, 0, 250, 0, 0, 0]).unwrap();
        }
        c.tick(50, &[0, 1]).unwrap();
        c
    }

    #[test]
    fn all_clients_restore_one_baseline_before_shared_activation() {
        let mut c = Coordinator::new(2, 2).unwrap();
        loaded(&mut c, 1);
        assert!(c.tick(0, &[1]).unwrap().is_empty());
        loaded(&mut c, 0);
        let clocks = c.tick(1, &[]).unwrap();
        assert_eq!(clocks.len(), 2);
        assert_eq!(clocks[0].body, [1, 0, 0, 0, 250, 0, 0, 0]);
        assert_eq!(clocks[0].body, clocks[1].body);
        assert_eq!(c.game_time(11), Ok(260));
        assert!(c.clock_reply(0, &[0; 8]).is_err());
        let a = c.clock_reply(0, &[1, 0, 0, 0, 250, 0, 0, 0]).unwrap();
        assert!(c.tick(40, &[0, 1]).unwrap().is_empty());
        let b = c.clock_reply(1, &[1, 0, 0, 0, 250, 0, 0, 0]).unwrap();
        assert_eq!(a[0].body, b[0].body);
        assert_eq!(&a[0].body[4..], empty(0).body);
        assert!(c.tick(40, &[0]).unwrap().is_empty());
        let events = c.tick(51, &[0, 1]).unwrap();
        assert_eq!(events.len(), 4);
        assert!(
            events
                .iter()
                .all(|e| e.opcode == 0x56 && read_u32(&e.body, 1) == 500)
        );
        assert!(c.tick(100, &[0, 1]).unwrap().is_empty());
        assert!(c.admit(0).is_err());
    }

    #[test]
    fn relay_has_authenticated_slot_and_deduplicates_old_revisions() {
        let mut c = running();
        let mut input = [0; 16];
        input[12..].copy_from_slice(&500u32.to_le_bytes());
        let (decision, deliveries) = c.input(0, &input, 250).unwrap();
        assert_eq!(decision, InputDecision::Accepted);
        assert_eq!(
            deliveries,
            vec![Delivery {
                slot: 1,
                opcode: 0x54,
                body: [vec![0], input.to_vec()].concat()
            }]
        );
        let original = input;
        input[8] = 1;
        assert_eq!(c.input(0, &input, 251).unwrap().0, InputDecision::Revision);
        assert_eq!(
            c.input(0, &original, 252).unwrap(),
            (InputDecision::Duplicate, vec![])
        );
        input[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(c.input(0, &input, 252).is_err());
        input[..4].fill(0);
        input[12..].copy_from_slice(&5000u32.to_le_bytes());
        assert_eq!(
            c.input(0, &input, 252).unwrap().0,
            InputDecision::OutsideWindow
        );
        assert_eq!(
            c.input(0, &original, 11000).unwrap().0,
            InputDecision::OutsideWindow
        );
        assert!(c.history.is_empty());
    }

    #[test]
    fn initial_state_boundaries_and_bootstrap_failure_are_explicit() {
        let mut bad = empty(0);
        bad.body.push(0);
        assert!(validate_empty(&bad).is_err());
        bad = empty(0);
        bad.body[4] = 255;
        assert!(validate_empty(&bad).is_err());
        bad = empty(0);
        bad.body[0] = 1;
        assert!(validate_empty(&bad).is_err());
        let mut c = Coordinator::new(2, 2).unwrap();
        loaded(&mut c, 0);
        loaded(&mut c, 1);
        c.tick(0, &[]).unwrap();
        assert!(c.tick(BOOTSTRAP_TIMEOUT + 1, &[]).is_err());
        assert!(c.remove(0, 100).is_err());
    }

    #[test]
    fn departure_is_shared_and_empty_match_can_restart() {
        let mut c = running();
        let leave = c.remove(0, 250).unwrap();
        assert_eq!(
            leave,
            vec![Delivery {
                slot: 1,
                opcode: 0x57,
                body: vec![0, 188, 2, 0, 0]
            }]
        );
        assert!(c.admit(0).is_err());
        c.remove(1, 300).unwrap();
        loaded(&mut c, 0);
        loaded(&mut c, 1);
        let clocks = c.tick(500, &[]).unwrap();
        assert_eq!(read_u32(&clocks[0].body, 0), 2);
        assert_eq!(c.game_time(500), Ok(250));
    }
}
