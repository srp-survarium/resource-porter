# Matched camera captures

Diagnostic tools for the pinned retail v0.10b and patched v0.26g executables.
They control the view at `base_game_scene::apply_camera` and capture the DXGI
backbuffer before Present. Original game archives are never patched by these
tools. Use local test servers and disposable executable copies.

1. Build `camera_hook.dll` with `bash build.sh` and an x86 MinGW compiler.
2. Run `python3 prepare_client.py --build 010b SOURCE_EXE NEW_EXE` (or `026g`).
   It checks the executable hash, appends a DLL import and startup stub, and
   refuses to overwrite an existing output. Put the DLL beside NEW_EXE.
3. Launch the copied game in its usual `binaries/win32` or `binaries/x86`
   directory with `SRP_CAPTURE_BUILD=010b|026g` and
   `SRP_CAPTURE_CONTROL=Z:\absolute\path\camera.txt` in its Wine environment.
   The startup stub initializes hooks outside the Windows loader lock.
4. Wait for the selected map to load. Confirm it in the game log and view;
   a successful capture alone does not prove that the correct map is running.
5. Use a 1280x720 client viewport in both games. v0.10b initially makes its
   outer window 1280x720, yielding a smaller backbuffer. Compile `viewport.c`
   to an x86 EXE and run it in that client's Wine prefix to correct this.
6. Run `python3 capture.py --control /path/camera.txt --poses poses.json
   --output /path/captures`. Repeat against the other client with the same
   pose file. Each BMP has a JSON record of the requested and applied camera.

The four supplied poses sample Factory's ground floor and upper area; their
names are identifiers, not verified architectural labels. Frame capture retains
the game's HUD and player model. Compare static scene regions away from those
features; moving the view does not teleport the player. Match weather, texture,
lighting, shading, shadow quality, and grading before attributing differences.
Allow exposure and streamed resources to settle after each camera move.

## Verified hook contract

| Build | Image base | apply_camera RVA | Camera pointer |
| --- | --- | --- | --- |
| 0.10b | 0x10000 | 0x5c72b0 | EDX |
| 0.26g | 0x10000 | 0x22ddd0 | EDX |

Both entries start `83 ec 40 53 8b 5c 24 48`; the hook checks these bytes before
writing. Inverse-view and projection matrices are at director +0x04 and +0x44.
These match the original v0.10b PDB/source and disassembly of both executables.
The shim preserves registers, flags, x87 and SSE state. Projection is D3D
left-handed with a vertical FOV; poses use world-space forward vectors.

The DLL writes `camera.txt.status.json` atomically. The controller changes the
request ID for each pose, waits for capture acknowledgment, and checks position.
The hook selects the first scene camera after each new request. Do not send a
request during loading or a scene transition. Unsupported backbuffer formats
and mismatched camera bytes fail explicitly. The supported capture formats are
single-sample RGBA8/BGRA8, including sRGB variants.

An initial CreateRemoteThread/WriteProcessMemory launcher was abandoned because
this host has `kernel.yama.ptrace_scope=2`. The current normal DLL import method
needs no change to that policy. Preserve the executable's embedded certificate:
both the blob and its PE directory entry are needed by v0.26 startup.

## Local reference runtime

The original v0.10b source registers `-client=HOST:PORT -spectator`, which connects
directly to a match server. It does not register `-localization`. A Russian UTF-8
locale avoids the legacy menu/font initialization failure seen with the default
locale. The local reference server is built from the existing legacy SRP source
with map name `level_03_rain` and port 2248. v0.26 uses map 182 at port 2247.
Restart the one-player 0.26 server with each client restart: its retained roster
can reject the next connection as full.

Runtime evidence is under the main checkout's `.work/render-compare/`.
The preserved artistic preset is separate:
`.work/lighting-presets/2026-10-13-atmosphere/README.md`.

## Intermediate lighting buffers (0.26 only)

Set `SRP_CAPTURE_BUFFERS=1` before launch to capture native-format GPU buffers
alongside each BMP. The hook first captures ambient inputs, then outputs, then
post-process inputs; Present acknowledges the screenshot only after these
captures finish. This stalls the GPU on requested frames only. It does not
change light values. Metadata lists actual target names, formats, dimensions,
write status and skipped resources. `.bin` files contain tightly packed mip-0
rows. `inspect_buffers.py IMAGE.bmp.targets.json` writes fixed-exposure previews
and statistics; two-channel previews display their first channel as grayscale.
Raw HDR values are not directly comparable between different engine contracts.

Verified against 0.21 symbols and the pinned 0.26 disassembly:

| Capture point | 0.26 VA | Stage pointer |
| --- | --- | --- |
| `apply_ssao_and_height_based_ambient` entry | 0x182530 | First stack argument |
| Immediately after that call in ambient execute | 0x1828bf | ESI |
| `stage_postprocess::execute` entry | 0x1b2270 | ECX |

Stage +4 points to context; context +0x119c points to the target array. Instances
have stride 0xa0, inline name +0x0c, and render-target pointer +0x98. The target's
D3D11 texture is at +0x0c. These offsets differ from 0.21 and must not be inferred
from its headers alone. Both ambient captures retain slots 31 (diffuse), 32
(ambient mask), and 34 (specular). Post-process capture includes the other named
lighting, normal, albedo, atmosphere and luminance buffers.

`python3 measure_ambient.py CAPTURE_FOLDER` reports per-pixel after/before ratios
for diffuse RGB sums above 0.0001. It measures attenuation within 0.26; it is not
a similarity score against 0.10b. Existing reference images in
`010b/poses-quality1` are frozen with `010b/reference-manifest.json`; further
iterations use only 0.26. A rendered map during countdown is sufficient.
