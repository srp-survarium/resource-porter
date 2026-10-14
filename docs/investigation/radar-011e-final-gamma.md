# Original 0.11e final gamma

Retain one explicit final `pow(color, 1/2.2)` in the compatibility recipe at the captured gamma setting of 1. Original `present.ps` is a passthrough because the earlier copy or post-sharpen pass already encodes gamma. Removing the compatibility gamma pass would introduce a new mismatch.

## Pinned evidence

The original executable SHA-256 is `83d6cd0728e3ab71acfa8507b9458c23373ac25ebace9ca4bf1539e53f9f2396`. Addresses below are virtual addresses in that executable (image base `0x10000`), identified with its matching PDB.

| Contract | Original executable evidence |
| --- | --- |
| Swapchain format | `res_render_output` constructor at `0x55705b` writes format 28, `R8G8B8A8_UNORM`. |
| Backbuffer view | `update_targets` at `0x556acc` calls `CreateRenderTargetView` with a null descriptor, inheriting UNORM rather than selecting an sRGB view. |
| Intermediate present target | `create_targets` at `0x6311b6` selects format 28; `0x6311bc` selects target `0x2e`. The actual name lookup jump table maps `0x2e` to `$user$present`. |
| Copy branch | Constructor at `0x608e83` creates `effect_copy_image` at member `+0xa0`; execute applies it at `0x6086b0` and draws to present at `0x608752`. |
| Sharpen branch | Constructor at `0x608ec7` creates `effect_post_process_sharpen` at member `+0xb0`; its second technique draws to present at `0x60869b`. |

The three archived pixel shaders establish the arithmetic independently of reconstructed source:

| Shader | SHA-256 | Relevant equation |
| --- | --- | --- |
| `copy_image.ps` | `685c00b3451a1fb3291f40696e7b4ae34b0b18f6bad32ab62e634b5f8f9cb451` | `pow(abs(color), 1/(gamma_correction_factor*2.2))` |
| `post_process_post_sharpen.ps` | `a9d9bdb481194a037e10f8cbd9f1d1d8aece9118d736e5f7f03965333b82e173` | `pow(abs(sharpened_color), 1/2.2)` |
| `present.ps` | `0a747779f1f1c05f61b8208fab09f784ab448c1d1d337bf22490cd9af85747dc` | One texture sample and return. |

`audit_011e_final_gamma.py` verifies the executable hash, instruction windows, actual target-name mapping, shader hashes, gamma instructions and constants. It extracts the three tiny bytecodes and writes `audit.json`. It needs only Python's standard library and reads the archive without launching Wine or the game:

```sh
python tools/render-compare/audit_011e_final_gamma.py \
  --exe /path/to/011e/app/binaries/win32/survarium.exe \
  --database /path/to/011e/app/resources.db \
  --listing /path/to/011e-list.txt \
  --output /path/to/gamma-audit
```

The listing is the existing `vostok-port` archive listing with length, offset and resource path fields. Validated output and selected disassembly are preserved at `/tmp/radar-011e-gamma-audit`; no large executable or archive copy is required.

## Limits

This proves where gamma is applied and rejects an sRGB-backbuffer explanation. It does not establish which AA branch was active in each screenshot, reproduce that branch's sharpening or filtering, prove display color management, or validate gamma settings other than the captured value 1. The post-tail core and ambient differences remain separate investigations. No shader, package or runtime setting was changed by this audit.
