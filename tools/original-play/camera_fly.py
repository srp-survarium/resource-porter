#!/usr/bin/env python3
"""Drive the 0.26 camera of a running hook session (free fly or typed poses).

    camera_fly.py RUNTIME                      interactive free fly (terminal)
    camera_fly.py RUNTIME pose X Y Z YAW PITCH jump to a pose and keep it
    camera_fly.py RUNTIME release              give the camera back to the game

RUNTIME is the session's runtime directory (it holds camera.txt and
camera.txt.status.json, written by the capture hook). The hook polls
camera.txt every 100 ms and, in mode 2, overrides the game camera every frame
with the requested pose. A fixed request id keeps it from taking more than the
first screenshot; `x` bumps the id to save one screenshot on purpose.

Pose convention: world metres, Y up; YAW in degrees, 0 = looking along +Z,
90 = along +X; PITCH in degrees, positive looks up.

Keys (interactive): w/s forward/back, a/d left/right, r/f up/down,
arrows (or j/l, i/k) turn, +/- speed, [/] field of view, p type a pose,
c copy the current pose into the status line, x screenshot, g release the
camera to the game (any move key takes it again), q quit (releases).
"""
import curses, json, math, os, sys, time
from pathlib import Path


class Camera:
    def __init__(self, runtime):
        self.runtime = Path(runtime)
        self.control = self.runtime/'camera.txt'
        self.id = int(time.time()) % 1000000 * 10 + 7
        self.pos = [0.0, 10.0, 0.0]; self.yaw = 0.0; self.pitch = 0.0
        self.fov = 65.0; self.speed = 2.0; self.held = False
        status = self.status()
        if status and any(status.get('position', [0, 0, 0])):
            self.pos = [float(v) for v in status['position']]
            f = status.get('forward', [0, 0, 1])
            self.yaw = math.degrees(math.atan2(f[0], f[2])); self.pitch = math.degrees(math.asin(max(-1, min(1, f[1]))))

    def status(self):
        try:
            return json.loads((self.runtime/'camera.txt.status.json').read_text())
        except (OSError, ValueError):
            return None

    def forward(self):
        y, p = math.radians(self.yaw), math.radians(self.pitch)
        return [math.sin(y)*math.cos(p), math.sin(p), math.cos(y)*math.cos(p)]

    def write(self, mode=2):
        f = self.forward()
        shot = 'Z:'+str(self.runtime/f'fly-{self.id}.bmp').replace('/', '\\')
        text = (f'{self.id} {mode} {self.pos[0]:.4f} {self.pos[1]:.4f} {self.pos[2]:.4f} '
                f'{f[0]:.6f} {f[1]:.6f} {f[2]:.6f} {self.fov:.3f} {16/9:.6f} 0.05 500 0\n{shot}\n')
        tmp = self.control.with_name('camera.txt.fly-tmp')
        tmp.write_text(text); os.replace(tmp, self.control)
        self.held = mode == 2

    def release(self):
        self.id += 1; self.write(mode=0)

    def pose(self):
        return f'{self.pos[0]:.2f} {self.pos[1]:.2f} {self.pos[2]:.2f} {self.yaw:.1f} {self.pitch:.1f}'


def interactive(screen, cam):
    curses.curs_set(0); screen.nodelay(True); screen.keypad(True)
    message = 'free fly: w/s a/d r/f move, arrows turn, p pose, x shot, g release, q quit'
    last = 0.0
    while True:
        key = screen.getch()
        moved = False
        if key in (ord('q'), 27):
            cam.release(); return
        f = cam.forward(); right = [math.cos(math.radians(cam.yaw)), 0, -math.sin(math.radians(cam.yaw))]
        step = cam.speed*0.25
        moves = {ord('w'): (f, 1), ord('s'): (f, -1), ord('d'): (right, 1), ord('a'): (right, -1),
                 ord('r'): ([0, 1, 0], 1), ord('f'): ([0, 1, 0], -1)}
        if key in moves:
            vec, sign = moves[key]
            cam.pos = [p+sign*step*v for p, v in zip(cam.pos, vec)]; moved = True
        elif key in (curses.KEY_LEFT, ord('j')): cam.yaw -= 4; moved = True
        elif key in (curses.KEY_RIGHT, ord('l')): cam.yaw += 4; moved = True
        elif key in (curses.KEY_UP, ord('i')): cam.pitch = min(89, cam.pitch+3); moved = True
        elif key in (curses.KEY_DOWN, ord('k')): cam.pitch = max(-89, cam.pitch-3); moved = True
        elif key in (ord('+'), ord('=')): cam.speed = min(50, cam.speed*1.5)
        elif key == ord('-'): cam.speed = max(0.1, cam.speed/1.5)
        elif key == ord('['): cam.fov = max(10, cam.fov-5); moved = True
        elif key == ord(']'): cam.fov = min(120, cam.fov+5); moved = True
        elif key == ord('x'):
            cam.id += 1; cam.write(); message = f'screenshot -> {cam.runtime}/fly-{cam.id}.bmp'
        elif key == ord('g'):
            cam.release(); message = 'camera released to the game; any move key takes it again'
        elif key == ord('c'):
            message = 'pose: '+cam.pose()
        elif key == ord('p'):
            curses.echo(); screen.nodelay(False)
            screen.addstr(6, 0, 'pose X Y Z YAW PITCH: '); screen.clrtoeol()
            text = screen.getstr(6, 22, 80).decode(errors='replace')
            curses.noecho(); screen.nodelay(True)
            try:
                x, y, z, yaw, pitch = (float(v) for v in text.replace(',', ' ').split()[:5])
                cam.pos = [x, y, z]; cam.yaw = yaw; cam.pitch = max(-89, min(89, pitch)); moved = True
                message = 'jumped to '+cam.pose()
            except ValueError:
                message = 'expected five numbers: X Y Z YAW PITCH'
        if moved or (cam.held and time.monotonic()-last > 1):
            cam.write(); last = time.monotonic()
        status = cam.status() or {}
        screen.erase()
        screen.addstr(0, 0, f'0.26 camera  runtime {cam.runtime}')
        screen.addstr(1, 0, f'pose {cam.pose()}   speed {cam.speed:.2f} m/key   fov {cam.fov:.0f}   {"held" if cam.held else "released"}')
        screen.addstr(2, 0, f'game: frames {status.get("present_calls", "?")}  camera calls {status.get("camera_calls", "?")}  error {status.get("error", "") or "-"}')
        screen.addstr(4, 0, message[:curses.COLS-1])
        screen.refresh(); time.sleep(0.02)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    cam = Camera(sys.argv[1])
    if not cam.runtime.is_dir():
        sys.exit(f'no runtime directory {cam.runtime}')
    if len(sys.argv) > 2 and sys.argv[2] == 'release':
        cam.release(); print('released'); return
    if len(sys.argv) > 2 and sys.argv[2] == 'pose':
        x, y, z, yaw, pitch = (float(v) for v in sys.argv[3:8])
        cam.pos = [x, y, z]; cam.yaw = yaw; cam.pitch = pitch; cam.write(); print('pose', cam.pose()); return
    curses.wrapper(interactive, cam)


if __name__ == '__main__':
    main()
