// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d surfaces (albedo, smoothness, metalness) as 0.26g draws surfaces
//! (diffuse, gloss, Fresnel), from the two builds' shader bytecode.
//!
//! 0.69d (`g_stage_default.ps`, `sun_lighting.ps`): the diffuse colour is
//! `A (1 - m)`, the Fresnel colour `F0 = 0.04 + m (A - 0.04)`, the GGX alpha
//! `max((1 - s)^2, 0.001)` of the smoothness `s = min + (max - min) * map.g`.
//!
//! 0.26g (`gbuffer_pass.ps`, `composition.ps` of shading quality 1 to 3):
//! a material's "roughness" is gloss `g`; the GGX alpha is `0.5 r^6 + 0.1 r`
//! of `r = 1 - g`, so it stops at 0.6. Fresnel is one number `f`: the diffuse
//! is drawn times `1 - f`, the specular times `f` and, above `f` 0.1, towards
//! the albedo's hue (`lerp(1.3 hue, 1, sat(1.04 - 4 f^2))`,
//! `hue = albedo / (sum albedo^4)^(1/4)`).
//!
//! Not equal in 0.26g whatever the values: its sun highlight carries 1/pi of
//! 0.69d's energy at the same alpha (the specular scale is a UNORM value, at
//! most 1), and an alpha above 0.6.
use anyhow::{Result, bail, ensure};

/// 0.69d's GGX alpha of a smoothness.
pub fn alpha_069(smoothness: f32) -> f32 {
    (1.0 - smoothness).powi(2).max(0.001)
}

/// 0.26g's GGX alpha of a gloss value.
pub fn alpha_026(gloss: f32) -> f32 {
    let r = (1.0 - gloss).clamp(0.0, 1.0);
    0.5 * r.powi(6) + 0.1 * r
}

/// The smoothness below which 0.69d is rougher than 0.26g can draw (alpha 0.6).
pub fn roughest() -> f32 {
    1.0 - alpha_026(0.0).sqrt()
}

/// The gloss with 0.69d's alpha: 0 below `roughest()`.
pub fn gloss_of(smoothness: f32) -> f32 {
    let want = alpha_069(smoothness);
    if want >= alpha_026(0.0) {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if alpha_026(mid) > want { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}

/// The smoothness up to which `gloss_line` follows `gloss_of`. The curve is
/// not a line (its slope runs from 0.5 to 3), so a smoothness map drawn as it
/// is, with new constants, follows it over a range only: from `roughest()`
/// to here (Rudnya's 118 smoothness maps: 96.8 % of the texels are at or
/// below 0.6). Above it the line is rougher than 0.69d (2.2 times at 0.8).
pub const LINE_TOP: f32 = 0.6;

/// `gloss = offset + slope * smoothness`, the least-squares line through
/// `gloss_of` between `roughest()` and `LINE_TOP`: about -0.17 + 0.69 s,
/// within 10 % of 0.69d's alpha there.
pub fn gloss_line() -> (f32, f32) {
    const N: usize = 64;
    let (lo, hi) = (roughest(), LINE_TOP);
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for i in 0..=N {
        let s = lo + (hi - lo) * i as f32 / N as f32;
        let (x, y) = (s as f64, gloss_of(s) as f64);
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let n = (N + 1) as f64;
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    (((sy - slope * sx) / n) as f32, slope as f32)
}

/// How a donor's gloss values become 0.26g's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gloss {
    /// The donor's value times a factor: 1 for a build with 0.26g's own gloss
    /// channel (0.46c2), or a manifest's own factor (`roughness_scale`).
    Scaled(f32),
    /// 0.69d smoothness, by `gloss_line`.
    Smoothness,
}

impl Gloss {
    /// An end of a gloss map's range (`constant_roughness_min` / `_max`). It
    /// may leave 0..1: the g-buffer clamps what it stores.
    pub fn end(self, value: f32) -> f32 {
        match self {
            Self::Scaled(k) => value * k,
            Self::Smoothness => {
                let (offset, slope) = gloss_line();
                offset + slope * value
            }
        }
    }

    /// A constant gloss.
    pub fn constant(self, value: f32) -> f32 {
        match self {
            Self::Scaled(k) => value * k,
            Self::Smoothness => gloss_of(value),
        }
    }
}

fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// 0.26g's specular tint of an albedo: `1.3 * albedo / (sum albedo^4)^(1/4)`
/// (`composition.ps` floors the root of the sum at 1e-4: a norm of 0.01).
fn tint(albedo: [f32; 3]) -> [f32; 3] {
    let norm = albedo.iter().map(|c| c.powi(4)).sum::<f32>().sqrt().max(1e-4).sqrt();
    albedo.map(|c| 1.3 * c / norm)
}

/// One 0.69d texel as 0.26g draws it: the linear diffuse colour and the
/// Fresnel value for a linear albedo and a metalness. At normal incidence the
/// diffuse light sent back is 0.69d's in each channel and the specular
/// light's luminance is 0.69d's; its colour is 0.69d's where the Fresnel is
/// above 0.51 (bare metal), white below 0.1.
pub fn metal_texel(albedo: [f32; 3], metalness: f32) -> ([f32; 3], f32) {
    let m = metalness.clamp(0.0, 1.0);
    let f0 = albedo.map(|a| 0.04 + m * (a - 0.04));
    let y = luminance(f0).clamp(0.0, 1.0);
    let body: [f32; 3] = std::array::from_fn(|c| albedo[c] * (1.0 - m) * (1.0 - f0[c]));
    if y <= 0.1 {
        return (body.map(|c| c / (1.0 - y)), y);
    }
    // The diffuse never goes black above 0.1, so the tint keeps a hue.
    let floor = tint(albedo).map(|c| c * 0.012 / 1.3);
    let hue = luminance(tint(std::array::from_fn(|c| body[c].max(floor[c]))));
    // f (t + (1 - t) hue) = y, t = sat(1.04 - 4 f^2): one root from y up.
    let shown = |f: f32| {
        let t = (1.04 - 4.0 * f * f).clamp(0.0, 1.0);
        f * (t + (1.0 - t) * hue)
    };
    let (mut lo, mut hi) = (y, 1.0f32);
    for _ in 0..32 {
        let mid = 0.5 * (lo + hi);
        if shown(mid) < y { lo = mid } else { hi = mid }
    }
    let f = (0.5 * (lo + hi)).min(0.98);
    (std::array::from_fn(|c| (body[c] / (1.0 - f)).max(floor[c])), f)
}

fn srgb_decode(v: u8) -> f32 {
    let v = v as f32 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let e = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (e * 255.0).round() as u8
}

// ---------------------------------------------------------------------------
// DXT1 / DXT5 colour blocks, level by level
// ---------------------------------------------------------------------------

/// A DXT1 or DXT5 texture's levels, as the archives hold them (a 128-byte
/// header, the mip chain, one colour-space byte).
struct Blocks<'a> {
    bytes: &'a [u8],
    width: usize,
    height: usize,
    levels: usize,
    /// 8 (DXT1) or 16 (DXT5: 8 bytes of alpha before the colour).
    block: usize,
}

impl<'a> Blocks<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        ensure!(bytes.len() > 128 && &bytes[..4] == b"DDS ", "not a DDS");
        let get = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
        let block = match &bytes[84..88] {
            b"DXT1" => 8,
            b"DXT5" => 16,
            other => bail!("unsupported texture format {:?}", String::from_utf8_lossy(other)),
        };
        let t = Self { bytes, width: get(16), height: get(12), levels: get(28).max(1), block };
        ensure!(get(112) & 0xfe00 == 0, "a cube map");
        ensure!(bytes.len() >= t.offset(t.levels), "truncated texture");
        Ok(t)
    }

    /// Blocks across and down at a level.
    fn grid(&self, level: usize) -> (usize, usize) {
        ((self.width >> level).max(1).div_ceil(4), (self.height >> level).max(1).div_ceil(4))
    }

    fn offset(&self, level: usize) -> usize {
        128 + (0..level).map(|l| self.grid(l).0 * self.grid(l).1 * self.block).sum::<usize>()
    }

    /// The level whose grid is closest to `across` blocks wide.
    fn level_like(&self, across: usize) -> usize {
        (0..self.levels).min_by_key(|l| self.grid(*l).0.abs_diff(across)).unwrap_or(0)
    }

    /// A block's 16 colours and which of them the block cuts out (DXT1's
    /// three-colour mode).
    fn colours(&self, level: usize, bx: usize, by: usize) -> ([[u8; 3]; 16], [bool; 16]) {
        let at = self.offset(level) + (by * self.grid(level).0 + bx) * self.block + (self.block - 8);
        let b = &self.bytes[at..at + 8];
        let (c0, c1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
        let (p0, p1) = (rgb565(c0), rgb565(c1));
        let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
        let four = self.block == 16 || c0 > c1;
        let palette: [[u8; 3]; 4] = if four {
            [p0, p1, std::array::from_fn(|i| mix(p0[i], p1[i], 2, 1)), std::array::from_fn(|i| mix(p0[i], p1[i], 1, 2))]
        } else {
            [p0, p1, std::array::from_fn(|i| mix(p0[i], p1[i], 1, 1)), [0; 3]]
        };
        let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        let index = |i: usize| ((bits >> (2 * i)) & 3) as usize;
        (std::array::from_fn(|i| palette[index(i)]), std::array::from_fn(|i| !four && index(i) == 3))
    }

    /// The 8 alpha bytes of a DXT5 block.
    fn alpha(&self, level: usize, bx: usize, by: usize) -> &'a [u8] {
        let at = self.offset(level) + (by * self.grid(level).0 + bx) * self.block;
        &self.bytes[at..at + 8]
    }
}

fn rgb565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) as u32 & 31, (c >> 5) as u32 & 63, c as u32 & 31);
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

fn to565(c: [f32; 3]) -> u16 {
    let q = |v: f32, top: f32| (v.clamp(0.0, 255.0) / 255.0 * top).round() as u16;
    q(c[0], 31.0) << 11 | q(c[1], 63.0) << 5 | q(c[2], 31.0)
}

/// A DXT1 colour block of 16 colours: the two ends of their spread along its
/// longest axis, each texel the nearest of the four (or, with cut-out texels,
/// three) palette colours.
fn colour_block(colours: &[[u8; 3]; 16], cut: &[bool; 16]) -> [u8; 8] {
    let kept: Vec<[f32; 3]> = (0..16).filter(|i| !cut[*i]).map(|i| colours[i].map(|c| c as f32)).collect();
    let any_cut = cut.iter().any(|c| *c);
    if kept.is_empty() {
        return [0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
    }
    let mean: [f32; 3] = std::array::from_fn(|c| kept.iter().map(|p| p[c]).sum::<f32>() / kept.len() as f32);
    // The longest axis: the channel ranges give its direction, signed by the
    // covariance with the widest channel.
    let range: [f32; 3] = std::array::from_fn(|c| kept.iter().map(|p| p[c]).fold(f32::MIN, f32::max) - kept.iter().map(|p| p[c]).fold(f32::MAX, f32::min));
    let widest = (0..3).max_by(|a, b| range[*a].total_cmp(&range[*b])).unwrap();
    let axis: [f32; 3] = std::array::from_fn(|c| {
        let cov: f32 = kept.iter().map(|p| (p[c] - mean[c]) * (p[widest] - mean[widest])).sum();
        if cov < 0.0 { -range[c] } else { range[c] }
    });
    let along = |p: &[f32; 3]| (0..3).map(|c| (p[c] - mean[c]) * axis[c]).sum::<f32>();
    let far = kept.iter().max_by(|a, b| along(a).total_cmp(&along(b))).unwrap();
    let near = kept.iter().min_by(|a, b| along(a).total_cmp(&along(b))).unwrap();
    let (mut c0, mut c1) = (to565(*far), to565(*near));
    // Four colours need c0 > c1; three and a cut-out c0 <= c1.
    if (c0 < c1) != any_cut {
        std::mem::swap(&mut c0, &mut c1);
    }
    let (p0, p1) = (rgb565(c0).map(|c| c as f32), rgb565(c1).map(|c| c as f32));
    let palette: Vec<[f32; 3]> = if any_cut || c0 == c1 {
        vec![p0, p1, std::array::from_fn(|i| (p0[i] + p1[i]) / 2.0)]
    } else {
        vec![p0, p1, std::array::from_fn(|i| (2.0 * p0[i] + p1[i]) / 3.0), std::array::from_fn(|i| (p0[i] + 2.0 * p1[i]) / 3.0)]
    };
    let mut bits = 0u32;
    for i in 0..16 {
        let index = if cut[i] {
            3
        } else {
            let p = colours[i].map(|c| c as f32);
            let distance = |q: &[f32; 3]| (0..3).map(|c| (p[c] - q[c]).powi(2)).sum::<f32>();
            (0..palette.len()).min_by(|a, b| distance(&palette[*a]).total_cmp(&distance(&palette[*b]))).unwrap() as u32
        };
        bits |= index << (2 * i);
    }
    let mut out = [0u8; 8];
    out[..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..].copy_from_slice(&bits.to_le_bytes());
    out
}

/// A metal surface's two 0.26g maps made from 0.69d's albedo and metalness
/// maps (`metal_texel` texel by texel, every level of the albedo's chain):
/// the diffuse map, in the albedo's format with its alpha as it is, and the
/// Fresnel map (its value in every channel), in the same layout.
pub struct MetalMaps {
    pub diffuse: Vec<u8>,
    pub fresnel: Vec<u8>,
    /// The Fresnel map's mean, for a template that draws one constant.
    pub mean_fresnel: f32,
}

/// A metal surface's metalness: 0.69d's map (a DDS), or one value for every
/// texel.
pub enum MetalInput {
    Map(Vec<u8>),
    Constant(f32),
}

/// `colour`: the material's linear albedo multiplier (0.69d's
/// `albedo_color^2.2`). A map's metalness is its green as it is, as 0.69d
/// reads it.
pub fn metal_maps(albedo: &[u8], metalness: &MetalInput, colour: [f32; 3]) -> Result<MetalMaps> {
    // The metalness as read: a map's blocks and its bytes, or the value.
    enum Read<'a> {
        Map(Blocks<'a>, &'a [u8]),
        Constant(f32),
    }
    let a = Blocks::new(albedo)?;
    let m = match metalness {
        MetalInput::Map(map) => Read::Map(Blocks::new(map)?, map),
        MetalInput::Constant(value) => Read::Constant(value.clamp(0.0, 1.0)),
    };
    let end = a.offset(a.levels);
    let mut diffuse = albedo[..128].to_vec();
    let mut fresnel = albedo[..128].to_vec();
    let (mut sum, mut count) = (0.0f64, 0usize);
    for level in 0..a.levels {
        let (across, down) = a.grid(level);
        for by in 0..down {
            for bx in 0..across {
                let (colours, cut) = a.colours(level, bx, by);
                // The metalness map's block at the same place (the maps are
                // one layout; a map of another size is read at its nearest
                // level and block).
                let metal: [f32; 16] = match &m {
                    Read::Map(m, _) => {
                        let like = m.level_like(across);
                        let (m_across, m_down) = m.grid(like);
                        let (metal, _) = m.colours(like, bx * m_across / across, by * m_down / down);
                        let mean = (metal.iter().map(|t| t[1] as u32).sum::<u32>() / 16) as u8;
                        let same = m_across == across && m_down == down;
                        std::array::from_fn(|i| if same { metal[i][1] } else { mean } as f32 / 255.0)
                    }
                    Read::Constant(value) => [*value; 16],
                };
                let mut d = [[0u8; 3]; 16];
                let mut f = [[0u8; 3]; 16];
                for i in 0..16 {
                    let linear: [f32; 3] = std::array::from_fn(|c| srgb_decode(colours[i][c]) * colour[c]);
                    let (body, value) = metal_texel(linear, metal[i]);
                    d[i] = body.map(srgb_encode);
                    f[i] = [(value * 255.0).round() as u8; 3];
                    if level == 0 && !cut[i] {
                        sum += value as f64;
                        count += 1;
                    }
                }
                if a.block == 16 {
                    diffuse.extend_from_slice(a.alpha(level, bx, by));
                    // An opaque alpha block.
                    fresnel.extend_from_slice(&[0xff, 0xff, 0, 0, 0, 0, 0, 0]);
                }
                diffuse.extend_from_slice(&colour_block(&d, &cut));
                fresnel.extend_from_slice(&colour_block(&f, &[false; 16]));
            }
        }
    }
    // The colour-space byte: the diffuse keeps the albedo's, the Fresnel map
    // is linear data like the metalness map.
    diffuse.extend_from_slice(&albedo[end..]);
    match &m {
        Read::Map(m, map) => fresnel.extend_from_slice(&map[m.offset(m.levels)..]),
        // Linear (0), as 0.26g's own Fresnel maps are.
        Read::Constant(_) => fresnel.push(0),
    }
    Ok(MetalMaps { diffuse, fresnel, mean_fresnel: (sum / count.max(1) as f64) as f32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gloss_follows_the_two_alphas() {
        // 0.26g cannot be rougher than alpha 0.6: 0.69d's smoothness 0.2254.
        assert!((roughest() - 0.2254).abs() < 1e-3 && gloss_of(0.1) == 0.0);
        for (s, g) in [(0.3, 0.039), (0.5, 0.167), (0.7, 0.381), (0.9, 0.900)] {
            assert!((gloss_of(s) - g).abs() < 2e-3, "{s}: {}", gloss_of(s));
            assert!((alpha_026(gloss_of(s)) / alpha_069(s) - 1.0).abs() < 1e-3);
        }
        // The line: about -0.17 + 0.69 s, within 10 % of the alpha up to 0.6.
        let (offset, slope) = gloss_line();
        assert!((offset + 0.17).abs() < 0.015 && (slope - 0.69).abs() < 0.03, "{offset} {slope}");
        for i in 0..=20 {
            let s = roughest() + (LINE_TOP - roughest()) * i as f32 / 20.0;
            let a = alpha_026(Gloss::Smoothness.end(s));
            assert!((a / alpha_069(s) - 1.0).abs() < 0.10, "{s}: {a} {}", alpha_069(s));
        }
        // A constant takes the curve itself; a factor stays a factor.
        assert!((Gloss::Smoothness.constant(0.9) - 0.9).abs() < 2e-3);
        assert_eq!((Gloss::Scaled(0.2).end(0.5), Gloss::Scaled(0.2).constant(0.5)), (0.1, 0.1));
    }

    #[test]
    fn a_metal_texel_sends_back_the_donors_light() {
        // A dielectric: the albedo, Fresnel 0.04.
        let (d, f) = metal_texel([0.5, 0.3, 0.2], 0.0);
        assert!((f - 0.04).abs() < 1e-6 && (d[0] - 0.5).abs() < 1e-5 && (d[2] - 0.2).abs() < 1e-5);
        // A dark metal (the pipe: albedo 0.05): no diffuse, Fresnel 0.05, where
        // the raw metalness as Fresnel reflected twenty times that.
        let (d, f) = metal_texel([0.05; 3], 1.0);
        assert!((f - 0.05).abs() < 1e-4 && d.iter().all(|c| *c < 1e-6), "{d:?} {f}");
        // Half metal: each channel's diffuse times (1 - f) is 0.69d's A (1 - m) (1 - F0).
        let a = [0.4, 0.25, 0.1];
        let (d, f) = metal_texel(a, 0.5);
        for c in 0..3 {
            let f0 = 0.04 + 0.5 * (a[c] - 0.04);
            assert!((d[c] * (1.0 - f) - a[c] * 0.5 * (1.0 - f0)).abs() < 1e-4, "{d:?} {f}");
        }
        // Bright bare metal: the specular luminance f * lerp(tint, 1, t) is F0's.
        let a = [0.9, 0.7, 0.3];
        let (d, f) = metal_texel(a, 1.0);
        let t = (1.04 - 4.0 * f * f).clamp(0.0, 1.0);
        let shown = f * (t + (1.0 - t) * luminance(tint(d)));
        assert!(f > 0.51 && (shown - luminance(a)).abs() < 5e-3, "{f} {shown}");
    }

    fn dxt1(colour: [u8; 3], levels: u32) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[..4].copy_from_slice(b"DDS ");
        for (at, v) in [(12, 8u32), (16, 8), (28, levels)] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[84..88].copy_from_slice(b"DXT1");
        let c = to565(colour.map(|v| v as f32));
        for level in 0..levels as usize {
            for _ in 0..((8usize >> level).max(1).div_ceil(4)).pow(2) {
                b.extend_from_slice(&c.to_le_bytes());
                b.extend_from_slice(&c.to_le_bytes());
                b.extend_from_slice(&[0; 4]);
            }
        }
        b.push(1);
        b
    }

    #[test]
    fn metal_maps_keep_the_albedos_layout() {
        let (albedo, metal) = (dxt1([128, 128, 128], 4), dxt1([255, 255, 255], 4));
        let maps = metal_maps(&albedo, &MetalInput::Map(metal), [1.0; 3]).unwrap();
        assert_eq!((maps.diffuse.len(), maps.fresnel.len()), (albedo.len(), albedo.len()));
        // A constant metalness of 1 makes the same maps but a linear Fresnel map.
        let constant = metal_maps(&albedo, &MetalInput::Constant(1.0), [1.0; 3]).unwrap();
        assert_eq!(constant.diffuse, maps.diffuse);
        assert_eq!((&constant.fresnel[..constant.fresnel.len() - 1], constant.fresnel.last()), (&maps.fresnel[..maps.fresnel.len() - 1], Some(&0)));
        assert_eq!(&maps.diffuse[..128], &albedo[..128]);
        // Grey 128 is 0.216 linear: all metal, so no diffuse beyond the hue's
        // floor and a Fresnel of about 0.22.
        let d = Blocks::new(&maps.diffuse).unwrap().colours(0, 1, 1).0[5];
        let f = Blocks::new(&maps.fresnel).unwrap().colours(0, 0, 0).0[0];
        assert!(d.iter().all(|c| *c < 40), "{d:?}");
        assert!((maps.mean_fresnel - 0.22).abs() < 0.02 && (f[1] as f32 / 255.0 - maps.mean_fresnel).abs() < 0.02, "{f:?} {}", maps.mean_fresnel);
        // A block of two colours comes back as those two.
        let mut colours = [[200u8, 40, 40]; 16];
        colours[3] = [16, 16, 200];
        let block = colour_block(&colours, &[false; 16]);
        let mut texture = dxt1([0; 3], 1);
        texture.truncate(128);
        texture.extend_from_slice(&[block, block, block, block].concat());
        texture.push(1);
        let back = Blocks::new(&texture).unwrap().colours(0, 0, 0).0;
        assert!(back[0][0] > 180 && back[3][2] > 180 && back[3][0] < 40, "{back:?}");
    }
}
