# Actual player camera captures

Request mode3 saves the current backbuffer without writing the director's view
or projection and without arming rich buffer readbacks. Modes0/1/2 retain their
existing follow-player/fixed-rich/fixed-screenshot behavior. The rain status
reports director control only for modes1/2.

Factory runtime batches `pipeline-factory-character-movement`, `-v3`, `-v4`
and `-v5` each captured two 1280×720 actual-player views. The current combined
cloud/diffuse/rain recipe rendered with empty hook errors, zero rain rejections
and director control disabled. The player's standing eye was approximately
`[-35.85087, 1.99374, -74.44600]`. These are screenshot-mode checks, not traversal
validation. `status.position` is the latest logic-camera position at status
publication; it is not a synchronized GPU matrix for exact image comparisons.

The `capture_gameplay.py` runner attempts four bounded WASD inputs and retains
partial results on failure. Its exact-executable Win32 helper rejects ambiguous
windows, already-held keys and lost foreground ownership. It pumps window
messages and is compiled as a GUI executable to avoid creating a console.
An isolated owned Win32 window received both W-down and W-up in a one-second
test, but Factory still refused activation: target Wine PID216, foreground
PID224 with an empty title, helper PID692. No movement input was sent in the
failed Factory attempts. Compositor selection by the unique executable app ID
did not resolve this. Niri exposes the Xwayland bridge PID for these windows,
so that PID cannot identify the Wine client process.

Movement remains unverified. Diagnose the game's foreground/window state
before retrying; do not weaken the helper's ownership guard. The runner's
position-displacement and advancing rain-counter checks must pass before a
batch is called a movement test. Its bounded helper normally sends key-up
before exiting; a forced helper termination is not a tested input-cleanup path.

The C tools compile with `-Wall -Wextra -Werror`; Python syntax checks pass.
No source-game reference captures were repeated for this diagnostic.
