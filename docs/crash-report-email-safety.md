# Build 802 crash-report email safety patch

The v0.100b/Windows build 802 client configures BugTrap with the retired
recipient `game_crash_reports@survarium.com`. The address is compiled into
`survarium.exe`; it is not in `resources.db`, the loose `resources/` tree, or
`bugtrap.dll`.

`tools/patch-crash-report-email.pl` changes the executable's single recipient
to `crash-reports@localhost.invalid`. The `.invalid` top-level domain is
reserved for names that must not resolve, so the replacement cannot designate
a real third-party mailbox. BugTrap still creates local error-report ZIP files.

For the stock build 802 executable:

- file size: `12235264`;
- original SHA-256:
  `169d42c0fc25bddc285f5e93e26a2769edbdbf3d50b9e0575d9eed4757b74b0b`;
- patched SHA-256:
  `f7459d65611d20fcafc8b1db5e3fa3aae1013c22238c779787a79e70e4fd50f7`;
- file offset: `0x943af8` (`.rdata`, virtual address `0x9548f8`);
- original 32 bytes: ASCII `game_crash_reports@survarium.com`;
- replacement 32 bytes: ASCII `crash-reports@localhost.invalid` followed by
  `00`.

The patcher validates the stock hash, file size, offset, and unique original
recipient before modifying the image. It writes atomically, retains
`survarium.exe.original-crash-email.bak`, and is idempotent. The demo launcher
runs it before starting SRP or Wine and refuses an unknown image.

Apply or verify it manually:

```sh
./tools/patch-crash-report-email.pl \
  --target .work/client-v0100b-akmn/binaries/win32/survarium.exe

./tools/patch-crash-report-email.pl --check \
  --target .work/client-v0100b-akmn/binaries/win32/survarium.exe
```
