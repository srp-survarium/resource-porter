// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's sky and fog colour: its precomputed atmosphere, evaluated here.
//!
//! 0.69d runs Bruneton's 2017 model (exe 0x2005560A0, 0x200555B50) with its
//! own tables: 2D, over (mu, r) only, for the current sun. The view of each
//! texel lies in the sun's vertical plane, so the tables know the sun's
//! elevation but not the azimuth of a view; the consumers add the phase
//! functions for the real angle. Tables (sizes from each program's texel
//! mapping): transmittance 256 x 64, irradiance 64 x 16, scattering 256 x 64
//! (u < 0.5 rays that hit the ground, u >= 0.5 rays that leave the top).
//! Spectral: 5 passes of 3 wavelengths over 360..830 nm, each converted to
//! linear sRGB by the CIE functions and summed (orders 1 to 4 in each), then
//! the transmittance once more at 680, 550, 440 nm for the consumers. Each
//! pass clears its target (`clear_to_zero`) and every target blends ONE +
//! ONE (exe 0x200536E0B), so the scattering table sums orders and passes.
//!
//! Clouds (`sky_dome.ps` variants with t3) need more than the table: the
//! cloud panorama, the dome's texture coordinates and its rotation phase.
//! Not modelled. The sun disc is not modelled either.
//!
//! Not read: the formats of the two kept tables (the temporaries are format
//! code 10, exe 0x2004CF9ED) and the sampler (taken as f32 and bilinear with
//! clamped edges, as Bruneton's).
//!
//! Against the donor's global probe (`rudnya_probe_check`) this sky is 1.8
//! to 1.9 times the probe away from the sun in all three channels (its
//! colour agrees within 10 %), 1.0 to 1.4 toward it: not explained yet.

// Planet (every program's immediates): ground and top radius (km), and
// H = sqrt(TOP^2 - BOTTOM^2).
const BOTTOM: f32 = 6360.0;
const TOP: f32 = 6420.0;
const BOTTOM2: f32 = 40_449_600.0;
const TOP2: f32 = 41_216_400.0;
const H: f32 = 875.671_143;
const H2: f32 = 766_800.0;

/// Texture sizes, from each program's `v0` scale: 1/256 and 1/64
/// (`calculate_atmospheric_transmittance.ps`, `calculate_single_scattering.ps`
/// 1/128 per half), 1/64 and 1/16 (`calculate_direct_irradiance.ps`).
pub const SCATTERING_SIZE: (usize, usize) = (256, 64);
pub const TRANSMITTANCE_SIZE: (usize, usize) = (256, 64);
pub const IRRADIANCE_SIZE: (usize, usize) = (64, 16);

/// A quantity at the three wavelengths of one spectral pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spectral(pub [f32; 3]);

/// Linear sRGB: a spectral quantity integrated against the CIE functions,
/// without the 683 lm/W (`sky_dome.ps` multiplies by 683).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgb(pub [f32; 3]);

/// Luminance in cd/m^2, per sRGB channel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Luminance(pub [f32; 3]);

/// One texel of 0.69d's scattering table: Rayleigh single and all multiple
/// scattering (without the phase function), and the red channel of single
/// Mie scattering (Bruneton's combined texture).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScatteringTexel {
    pub rayleigh: Rgb,
    pub mie_red: f32,
}

pub trait Texel: Copy + Default + Send + Sync {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

impl Texel for Spectral {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        Spectral(mix3(a.0, b.0, t))
    }
}

impl Texel for (Spectral, Spectral) {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        (Spectral::lerp(a.0, b.0, t), Spectral::lerp(a.1, b.1, t))
    }
}

impl Texel for ScatteringTexel {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        ScatteringTexel { rayleigh: Rgb(mix3(a.rayleigh.0, b.rayleigh.0, t)), mie_red: a.mie_red + (b.mie_red - a.mie_red) * t }
    }
}

/// A 2D table, row-major, sampled bilinearly with clamped edges.
#[derive(Clone, Debug)]
pub struct Table<T> {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<T>,
}

impl<T: Texel> Table<T> {
    /// Fills the table with `f(x, y)`, rows spread over a few threads.
    fn build((width, height): (usize, usize), f: impl Fn(usize, usize) -> T + Sync) -> Self {
        let mut texels = vec![T::default(); width * height];
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8);
        let rows = height.div_ceil(threads);
        std::thread::scope(|s| {
            for (k, chunk) in texels.chunks_mut(rows * width).enumerate() {
                let f = &f;
                s.spawn(move || {
                    for (o, t) in chunk.iter_mut().enumerate() {
                        let i = k * rows * width + o;
                        *t = f(i % width, i / width);
                    }
                });
            }
        });
        Table { width, height, texels }
    }

    pub fn texel(&self, x: usize, y: usize) -> T {
        self.texels[y * self.width + x]
    }

    pub fn sample(&self, u: f32, v: f32) -> T {
        let axis = |c: f32, n: usize| {
            let p = c * n as f32 - 0.5;
            let f = p.floor();
            let i0 = (f as isize).clamp(0, n as isize - 1) as usize;
            let i1 = (f as isize + 1).clamp(0, n as isize - 1) as usize;
            (i0, i1, p - f)
        };
        let (x0, x1, fx) = axis(u, self.width);
        let (y0, y1, fy) = axis(v, self.height);
        let top = T::lerp(self.texel(x0, y0), self.texel(x1, y0), fx);
        let bottom = T::lerp(self.texel(x0, y1), self.texel(x1, y1), fx);
        T::lerp(top, bottom, fy)
    }
}

impl<T: Texel> Table<T> {
    fn zip(&self, other: &Table<T>, f: impl Fn(T, T) -> T) -> Table<T> {
        Table { width: self.width, height: self.height, texels: self.texels.iter().zip(&other.texels).map(|(&a, &b)| f(a, b)).collect() }
    }
}

// Shader arithmetic. Constants are the programs' immediates (exact floats
// of the bytecode); `exp(x)` there is `exp2(-1.44269502 x)`.
const LOG2E: f32 = 1.442_695;

fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let n = dot(a, a).sqrt().recip();
    a.map(|v| v * n)
}

/// Density profiles at altitude `h` km: Rayleigh `exp(-h / 8)`, Mie
/// `exp(-h / 1.2)`, ozone a tent from 10 to 40 km peaking at 25
/// (`calculate_*.ps`: exp2 of -0.180336878 h and -1.20224583 h; the tent's
/// 0.0666666701, -0.666666687, 2.66666675).
fn densities(h: f32) -> [f32; 3] {
    let ozone = if h < 25.0 { sat(h * 0.066_666_67 - 0.666_666_7) } else { sat(h * -0.066_666_67 + 2.666_666_7) };
    [(h * -0.180_336_88).exp2(), (h * -1.202_245_8).exp2(), ozone]
}

/// `0.0891267657 + 0.0318309888 nu`: 0.69d's "Rayleigh" phase function
/// (every program that applies one).
fn rayleigh_phase(nu: f32) -> f32 {
    nu * 0.031_830_99 + 0.089_126_77
}

/// Cornette-Shanks with g = 0.76: `0.0195609424 (1 + nu^2) / (1.5776 -
/// 1.52 nu)^1.5`.
fn mie_phase(nu: f32) -> f32 {
    0.019_560_942 * (nu * nu + 1.0) / ((-nu * 1.52 + 1.5776).log2() * 1.5).exp2()
}

/// `r^2 - BOTTOM^2` from the altitude `h = r - BOTTOM`, the programs' form
/// (`h^2 + 12720 h`).
fn rho2(h: f32) -> f32 {
    h * h + h * 12_720.0
}

/// Transmittance table coordinates of (r, mu): the distance to the top
/// against its range (`calculate_direct_irradiance.ps`, and every reader).
fn transmittance_uv(r: f32, mu: f32) -> (f32, f32) {
    let rho = rho2(r - BOTTOM).sqrt();
    let d = ((r * r * (mu * mu - 1.0) + TOP2).max(0.0).sqrt() - r * mu).max(0.0);
    let d_min = TOP - r;
    let u = (d - d_min) / (rho + H - d_min) * 0.996_093_75 + 0.001_953_125;
    (u, rho * 0.001_124_137_8 + 0.007_812_5)
}

/// Where a ray from r in direction mu ends, which picks the table half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RayEnd {
    Ground,
    Top,
}

/// Scattering table coordinates of (r, mu) for a ray ending at `end`
/// (`calculate_scattering_density.ps`, `calculate_multiple_scattering.ps`,
/// `sky_dome.ps`, `sun_lighting.ps`: the same instructions).
fn scattering_uv(r: f32, mu: f32, end: RayEnd) -> (f32, f32) {
    let h = r - BOTTOM;
    let rho2 = rho2(h);
    let rho = rho2.sqrt();
    let disc = rho2 * (mu * mu - 1.0) + mu * mu * BOTTOM2;
    let u = match end {
        RayEnd::Ground if rho == h => 0.498_046_88,
        RayEnd::Ground => {
            let d = -r * mu - disc.max(0.0).sqrt();
            -(((d - h) / (rho - h)) * 0.992_187_5 + 0.003_906_25) * 0.5 + 0.5
        }
        RayEnd::Top => {
            let d_min = TOP - r;
            let d = -r * mu + (disc + H2).max(0.0).sqrt();
            ((d - d_min) / (rho + H - d_min) * 0.992_187_5 + 0.003_906_25) * 0.5 + 0.5
        }
    };
    (u, rho * 0.001_124_137_8 + 0.007_812_5)
}

/// Where a ray really ends: the ground if it points down and meets it.
fn ray_end(r: f32, mu: f32) -> RayEnd {
    let disc = rho2(r - BOTTOM) * (mu * mu - 1.0) + mu * mu * BOTTOM2;
    if mu < 0.0 && disc >= 0.0 { RayEnd::Ground } else { RayEnd::Top }
}

/// The (r, mu) of scattering texel (x, y) and its half
/// (`calculate_single_scattering.ps` and the two later programs, first 33
/// instructions).
fn scattering_texel(x: usize, y: usize) -> (f32, f32, RayEnd) {
    let px = x as f32 + 0.5;
    let fy = (y as f32 + 0.5) * 0.015_625 - 0.007_812_5;
    let fx = px * 0.007_812_5 - 1.003_906_3;
    let xr = fy * 1.015_873_1;
    let rho = fy * 889.570_74;
    let r = (xr * xr * H2 + BOTTOM2).sqrt();
    if px < 128.0 {
        let d_min = r - BOTTOM;
        let d = (rho - d_min) * (-px * 0.007_812_5 + 0.996_093_75) * 1.007_874 + d_min;
        let mu = if d == 0.0 { -1.0 } else { (-(d * d + rho * rho) / (d * (r + r))).clamp(-1.0, 1.0) };
        (r, mu, RayEnd::Ground)
    } else {
        let d_min = TOP - r;
        let d = (fy * 889.570_74 + H - d_min) * fx * 1.007_874 + d_min;
        let mu = if d == 0.0 { 1.0 } else { ((H2 - rho * rho - d * d) / (d * (r + r))).clamp(-1.0, 1.0) };
        (r, mu, RayEnd::Top)
    }
}

/// Distance along (r, mu) to where the texel's ray ends.
fn ray_length(r: f32, mu: f32, end: RayEnd) -> f32 {
    match end {
        RayEnd::Ground => (-r * mu - (rho2(r - BOTTOM) * (mu * mu - 1.0) + mu * mu * BOTTOM2).max(0.0).sqrt()).max(0.0),
        RayEnd::Top => (-r * mu + (r * r * (mu * mu - 1.0) + TOP2).max(0.0).sqrt()).max(0.0),
    }
}

/// The view of a scattering texel: zenith cosine `mu` in the vertical
/// plane toward the sun (`-light.xz`; straight up the sun, any plane).
fn texel_view(mu: f32, light: [f32; 3]) -> [f32; 3] {
    let (hx, hz) = if -0.999 < light[1] { (-light[0], -light[2]) } else { (0.0, 1.0) };
    let y = if 0.999 < mu.abs() { mu } else { mu * (hx * hx + hz * hz).sqrt() / (1.0 - mu * mu).sqrt() };
    normalize([hx, y, hz])
}

/// What the precomputation programs read from
/// `atmosphere_precomputation_constants` (filled at exe 0x200555E80).
#[derive(Clone, Copy, Debug)]
struct PassConstants {
    /// m_params_0.xyz: solar irradiance at the pass's wavelengths times
    /// `sun_intensity_r2`.
    solar: Spectral,
    /// m_params_1.xyz: Rayleigh scattering (1/km).
    rayleigh: Spectral,
    /// m_params_1.w: the sun's angular radius (rad).
    sun_angular_radius: f32,
    /// m_params_2.xyz: ozone absorption (1/km).
    ozone: Spectral,
    /// m_params_2.w, m_params_3.w: Mie scattering and extinction (1/km).
    mie_scattering: f32,
    mie_extinction: f32,
    /// m_params_3.xyz: the direction sunlight travels.
    light: [f32; 3],
    /// m_params_4..6: the columns of the pass's spectral-to-sRGB matrix.
    to_rgb: [[f32; 3]; 3],
}

impl PassConstants {
    fn extinction(&self, depth: [f32; 3]) -> Spectral {
        Spectral(std::array::from_fn(|c| {
            ((depth[0] * self.rayleigh.0[c] + depth[1] * self.mie_extinction + depth[2] * self.ozone.0[c]) * -LOG2E).exp2()
        }))
    }

    fn rgb(&self, s: Spectral) -> Rgb {
        let m = &self.to_rgb;
        Rgb(std::array::from_fn(|k| s.0[0] * m[0][k] + s.0[1] * m[1][k] + s.0[2] * m[2][k]))
    }
}

/// `calculate_atmospheric_transmittance.ps`: 513 trapezoid samples to the top.
fn transmittance(c: &PassConstants) -> Table<Spectral> {
    Table::build(TRANSMITTANCE_SIZE, |x, y| {
        let fx = (x as f32 + 0.5) * 0.003_906_25 - 0.001_953_125;
        let fy = (y as f32 + 0.5) * 0.015_625 - 0.007_812_5;
        let xmu = fx * 1.003_921_6;
        let rho = fy * 889.570_74;
        let r2 = rho * rho + BOTTOM2;
        let r = r2.sqrt();
        let d_min = TOP - r;
        let d = xmu * (fy * 889.570_74 + H - d_min) + d_min;
        let mu = if d == 0.0 { 1.0 } else { ((-rho * rho + H2 - d * d) / (d * (r + r))).clamp(-1.0, 1.0) };
        let dx = ((r2 * (mu * mu - 1.0) + TOP2).max(0.0).sqrt() - r * mu).max(0.0) * 0.001_953_125;
        let two_r_mu = mu * (r + r);
        let mut sum = [0.0f32; 3];
        for k in 0..=512u32 {
            let t = dx * k as f32;
            let h = (r2 + (t * t + t * two_r_mu)).sqrt() - BOTTOM;
            let [dr, dm, d_o] = densities(h);
            let w = if k == 0 || k == 512 { 0.5 } else { 1.0 };
            sum[0] += w * dr.min(1.0);
            sum[1] += w * dm.min(1.0);
            sum[2] += w * d_o;
        }
        c.extinction(sum.map(|s| s * dx))
    })
}

/// `calculate_direct_irradiance.ps`: the sun on a horizontal surface, a
/// disc of `sun_angular_radius` cut by the horizon.
fn direct_irradiance(c: &PassConstants, t: &Table<Spectral>) -> Table<Spectral> {
    Table::build(IRRADIANCE_SIZE, |x, y| {
        let fx = (x as f32 + 0.5) * 0.015_625 - 0.007_812_5;
        let fy = (y as f32 + 0.5) * 0.0625 - 0.031_25;
        let mu_s = (fx * 2.031_746_1 - 1.0).clamp(-1.0, 1.0);
        let r = fy * 64.0 + BOTTOM;
        let (u, v) = transmittance_uv(r, mu_s);
        let a = c.sun_angular_radius;
        let f = if mu_s < -a {
            0.0
        } else if a < mu_s {
            mu_s
        } else {
            (mu_s + a) * (mu_s + a) / (a * 4.0)
        };
        let tr = t.sample(u, v);
        Spectral(std::array::from_fn(|k| f * tr.0[k] * c.solar.0[k]))
    })
}

/// Single scattering of one texel (`calculate_single_scattering.ps`):
/// Rayleigh and Mie, without phase functions.
fn single_texel(c: &PassConstants, t: &Table<Spectral>, x: usize, y: usize) -> (Spectral, Spectral) {
    let (r, mu, end) = scattering_texel(x, y);
    let nu = dot(texel_view(mu, c.light), c.light.map(|v| -v));
    let mu_s = -c.light[1];
    let dx = ray_length(r, mu, end) * 0.007_812_5;
    let r2 = r * r;
    let two_r_mu = mu * (r + r);
    let (mut depth, mut sum_r, mut sum_m) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
    for k in 0..=128u32 {
        let d = dx * k as f32;
        let w = if k == 0 || k == 128 { 0.5 } else { 1.0 };
        let rt = (r2 + (d * d + two_r_mu * d)).sqrt().clamp(BOTTOM, TOP);
        let mu_st = ((d * nu + r * mu_s) / rt).clamp(-1.0, 1.0);
        let h = rt - BOTTOM;
        let dens = densities(h);
        let next: [f32; 3] = std::array::from_fn(|i| dens[i] * dx * w + depth[i]);
        // The view's transmittance from the depth before this sample.
        let view = c.extinction(depth);
        let s = BOTTOM / rt;
        let mu_h = -rho2(h).sqrt() / rt;
        let (u, v) = transmittance_uv(rt, mu_st);
        let sun = t.sample(u, v);
        let a = c.sun_angular_radius;
        let f = sat((mu_st - mu_h + s * a) * (2.0 * s * a).recip());
        let f = f * f * (f * -2.0 + 3.0);
        for i in 0..3 {
            let tt = f * sun.0[i] * view.0[i];
            sum_r[i] += w * (dens[0] * tt);
            sum_m[i] += w * (dens[1] * tt);
        }
        depth = next;
    }
    (
        Spectral(std::array::from_fn(|i| sum_r[i] * (dx * c.rayleigh.0[i]) * c.solar.0[i])),
        Spectral(std::array::from_fn(|i| sum_m[i] * (dx * c.mie_scattering) * c.solar.0[i])),
    )
}

/// The scattered light that arrives along a direction, as the density and
/// indirect irradiance programs read it: single scattering with phase
/// functions at order 2 (their `order - 1 == 1` test), else the previous
/// order's multiple scattering.
enum Incident<'a> {
    Single { rayleigh: &'a Table<Spectral>, mie: &'a Table<Spectral> },
    Multiple(&'a Table<Spectral>),
}

impl Incident<'_> {
    /// The three texels read at (u, v); `light(nu)` combines them.
    fn at(&self, u: f32, v: f32) -> IncidentTexel {
        match self {
            Incident::Single { rayleigh, mie } => IncidentTexel::Single(rayleigh.sample(u, v), mie.sample(u, v)),
            Incident::Multiple(m) => IncidentTexel::Multiple(m.sample(u, v)),
        }
    }
}

enum IncidentTexel {
    Single(Spectral, Spectral),
    Multiple(Spectral),
}

impl IncidentTexel {
    fn light(&self, nu: f32) -> Spectral {
        match self {
            IncidentTexel::Single(r, m) => {
                let (pr, pm) = (rayleigh_phase(nu), mie_phase(nu));
                Spectral(std::array::from_fn(|k| pm * m.0[k] + r.0[k] * pr))
            }
            IncidentTexel::Multiple(m) => *m,
        }
    }
}

/// `calculate_scattering_density.ps`: light scattered toward the texel's
/// view at its point, from 16 x 32 directions, the ground (albedo 0.1)
/// lit by `irradiance` at its foot.
fn scattering_density(c: &PassConstants, t: &Table<Spectral>, incident: &Incident, irradiance: &Table<Spectral>) -> Table<Spectral> {
    let to_sun = c.light.map(|v| -v);
    Table::build(SCATTERING_SIZE, |x, y| {
        let (r, mu, _) = scattering_texel(x, y);
        let view = texel_view(mu, c.light);
        let h = r - BOTTOM;
        let [dr, dm, _] = densities(h);
        let mut out = [0.0f32; 3];
        for m in 0..16u32 {
            let theta = (m as f32 + 0.5) * 0.196_349_55;
            let (sin_t, cos_t) = theta.sin_cos();
            let d_omega = sin_t * 0.038_553_14;
            // Ground seen this way: its distance, transmittance and albedo.
            let (dg, ground) = match ray_end(r, cos_t) {
                RayEnd::Ground => {
                    let dg = ray_length(r, cos_t, RayEnd::Ground);
                    let rg = (r * r + (dg * dg + dg * ((r + r) * cos_t))).sqrt().clamp(BOTTOM, TOP);
                    let mu_g = ((r * cos_t + dg) / rg).clamp(-1.0, 1.0);
                    let (u1, v1) = transmittance_uv(rg, -mu_g);
                    let (u2, v2) = transmittance_uv(r, -cos_t);
                    let (t1, t2) = (t.sample(u1, v1), t.sample(u2, v2));
                    (dg, Spectral(std::array::from_fn(|k| (t1.0[k] / t2.0[k]).min(1.0) * 0.1)))
                }
                RayEnd::Top => (0.0, Spectral::default()),
            };
            let (u, v) = scattering_uv(r, cos_t, ray_end(r, cos_t));
            let texel = incident.at(u, v);
            for n in 0..32u32 {
                let phi = (n as f32 + 0.5) * 0.196_349_55;
                let (sin_p, cos_p) = phi.sin_cos();
                // Game axes: y up.
                let omega = [sin_t * cos_p, cos_t, sin_t * sin_p];
                let inc = texel.light(dot(to_sun, omega));
                let normal = normalize([dg * omega[0], dg * omega[1] + r, dg * omega[2]]);
                let mu_sg = dot(normal, to_sun);
                let e = irradiance.sample((mu_sg * 0.5 + 0.5) * 0.984_375 + 0.007_812_5, 0.031_25);
                let nu2 = dot(view, omega);
                let pr = rayleigh_phase(nu2) * dr;
                let pm = mie_phase(nu2) * dm * c.mie_scattering;
                for k in 0..3 {
                    let l = ground.0[k] * e.0[k] * 0.318_309_87 + inc.0[k];
                    out[k] += l * (pr * c.rayleigh.0[k] + pm) * d_omega;
                }
            }
        }
        Spectral(out)
    })
}

/// `calculate_indirect_irradiance.ps`: the sky's light on a horizontal
/// surface, 16 x 64 directions over the upper hemisphere.
fn indirect_irradiance(incident: &Incident) -> Table<Spectral> {
    Table::build(IRRADIANCE_SIZE, |x, y| {
        let fx = (x as f32 + 0.5) * 0.015_625 - 0.007_812_5;
        let fy = (y as f32 + 0.5) * 0.0625 - 0.031_25;
        let mu_s = (fx * 2.031_746_1 - 1.0).clamp(-1.0, 1.0);
        let sin_s = (1.0 - mu_s * mu_s).sqrt();
        let r = fy * 64.0 + BOTTOM;
        let rho2 = (fy * 64.0) * (fy * 64.0) + fy * 814_080.0;
        let rho = rho2.max(0.0).sqrt();
        let v = rho * 0.001_124_137_8 + 0.007_812_5;
        let d_min = TOP - r;
        let mut out = [0.0f32; 3];
        for m in 0..16u32 {
            let theta = (m as f32 + 0.5) * 0.098_174_77;
            let (sin_t, cos_t) = theta.sin_cos();
            let d_omega = sin_t * 0.009_638_285;
            let d = (rho2 * (cos_t * cos_t - 1.0) + cos_t * cos_t * BOTTOM2 + H2).max(0.0).sqrt() - r * cos_t;
            let u = ((d - d_min) / (rho + H - d_min) * 0.992_187_5 + 0.003_906_25) * 0.5 + 0.5;
            let texel = incident.at(u, v);
            for n in 0..64u32 {
                let cos_p = ((n as f32 + 0.5) * 0.098_174_77).cos();
                let nu = sin_s * (sin_t * cos_p) + mu_s * cos_t;
                let l = texel.light(nu);
                for k in 0..3 {
                    out[k] += cos_t * l.0[k] * d_omega;
                }
            }
        }
        Spectral(out)
    })
}

/// `calculate_multiple_scattering.ps`: the density integrated along the
/// texel's ray; returns it and what is added to the scattering table
/// (sRGB, divided by the Rayleigh phase of the texel's view).
fn multiple_scattering(c: &PassConstants, density: &Table<Spectral>) -> (Table<Spectral>, Table<ScatteringTexel>) {
    let delta = Table::build(SCATTERING_SIZE, |x, y| {
        let (r, mu, end) = scattering_texel(x, y);
        let dx = ray_length(r, mu, end) * 0.007_812_5;
        let r2 = r * r;
        let two_r_mu = mu * (r + r);
        let (mut depth, mut sum) = ([0.0f32; 3], [0.0f32; 3]);
        for k in 0..=128u32 {
            let d = dx * k as f32;
            let w = if k == 0 || k == 128 { 0.5 } else { 1.0 };
            let rt = (r2 + (d * d + two_r_mu * d)).sqrt().clamp(BOTTOM, TOP);
            let mu_t = ((r * mu + d) / rt).clamp(-1.0, 1.0);
            let dens = densities(rt - BOTTOM);
            let next: [f32; 3] = std::array::from_fn(|i| dens[i] * dx * w + depth[i]);
            let view = c.extinction(depth);
            let (u, v) = scattering_uv(rt, mu_t, end);
            let j = density.sample(u, v);
            for i in 0..3 {
                sum[i] += view.0[i] * j.0[i] * w;
            }
            depth = next;
        }
        Spectral(sum.map(|s| s * dx))
    });
    let added = Table::build(SCATTERING_SIZE, |x, y| {
        let (_, mu, _) = scattering_texel(x, y);
        let pr = rayleigh_phase(dot(texel_view(mu, c.light), c.light.map(|v| -v)));
        ScatteringTexel { rayleigh: c.rgb(Spectral(delta.texel(x, y).0.map(|s| s / pr))), mie_red: 0.0 }
    });
    (delta, added)
}

/// Single scattering's share of the scattering table (its third target).
fn single_scattering(c: &PassConstants, t: &Table<Spectral>) -> (Table<Spectral>, Table<Spectral>, Table<ScatteringTexel>) {
    let both = Table::build(SCATTERING_SIZE, |x, y| single_texel(c, t, x, y));
    let map = |f: &dyn Fn((Spectral, Spectral)) -> Spectral| Table { width: both.width, height: both.height, texels: both.texels.iter().map(|&p| f(p)).collect() };
    let (rayleigh, mie) = (map(&|p| p.0), map(&|p| p.1));
    let combined = Table {
        width: both.width,
        height: both.height,
        texels: both.texels.iter().map(|&(r, m)| ScatteringTexel { rayleigh: c.rgb(r), mie_red: c.rgb(m).0[0] }).collect(),
    };
    (rayleigh, mie, combined)
}

// Spectra as 0.69d's initializer fills them (exe 0x2004d03cb..0x2004d0484):
// 48 wavelengths 360..830 nm in steps of 10; solar irradiance copied from
// 0x2009c7650; Rayleigh `1241090 lambda^-4` /m (0x2009e6008, exponent
// 0x2009e6200); ozone `5.374e18` /m^3 (0x2009e60b0) times the cross
// sections at 0x2009c68a0 (m^2). Bruneton's demo tables.
const SOLAR_IRRADIANCE: [f64; 48] = [
    1.11776, 1.14259, 1.01249, 1.14716, 1.72765, 1.73054, 1.6887, 1.61253,
    1.91198, 2.03474, 2.02042, 2.02212, 1.93377, 1.95809, 1.91686, 1.8298,
    1.8685, 1.8931, 1.85149, 1.8504, 1.8341, 1.8345, 1.8147, 1.78158,
    1.7533, 1.6965, 1.68194, 1.64654, 1.6048, 1.52143, 1.55622, 1.5113,
    1.474, 1.4482, 1.41018, 1.36775, 1.34188, 1.31429, 1.28303, 1.26758,
    1.2367, 1.2082, 1.18737, 1.14683, 1.12362, 1.1058, 1.07124, 1.04992,
];
const OZONE_CROSS_SECTION: [f64; 48] = [
    1.18e-27, 2.182e-28, 2.818e-28, 6.636e-28, 1.527e-27, 2.763e-27, 5.52e-27, 8.451e-27,
    1.582e-26, 2.316e-26, 3.669e-26, 4.924e-26, 7.752e-26, 9.016e-26, 1.48e-25, 1.602e-25,
    2.139e-25, 2.755e-25, 3.091e-25, 3.5e-25, 4.266e-25, 4.672e-25, 4.398e-25, 4.701e-25,
    5.019e-25, 4.305e-25, 3.74e-25, 3.215e-25, 2.662e-25, 2.238e-25, 1.852e-25, 1.473e-25,
    1.209e-25, 9.423e-26, 7.455e-26, 6.566e-26, 5.105e-26, 4.15e-26, 4.228e-26, 3.237e-26,
    2.451e-26, 2.801e-26, 2.534e-26, 1.624e-26, 1.465e-26, 2.078e-26, 1.383e-26, 7.105e-27,
];
const RAYLEIGH_AT_1NM: f64 = 1_241_090.0;
const OZONE_NUMBER_DENSITY: f64 = 5.374e18;

/// CIE 1931 2-degree colour matching functions (x, y, z), 360..830 nm in
/// steps of 5 (exe 0x2009c6a20), and XYZ to linear sRGB (0x2009c7600).
const CIE: [[f64; 3]; 95] = [
    [0.0001299, 3.917e-06, 0.0006061],
    [0.0002321, 6.965e-06, 0.001086],
    [0.0004149, 1.239e-05, 0.001946],
    [0.0007416, 2.202e-05, 0.003486],
    [0.001368, 3.9e-05, 0.006450001],
    [0.002236, 6.4e-05, 0.01054999],
    [0.004243, 0.00012, 0.02005001],
    [0.00765, 0.000217, 0.03621],
    [0.01431, 0.000396, 0.06785001],
    [0.02319, 0.00064, 0.1102],
    [0.04351, 0.00121, 0.2074],
    [0.07763, 0.00218, 0.3713],
    [0.13438, 0.004, 0.6456],
    [0.21477, 0.0073, 1.0390501],
    [0.2839, 0.0116, 1.3856],
    [0.3285, 0.01684, 1.62296],
    [0.34828, 0.023, 1.74706],
    [0.34806, 0.0298, 1.7826],
    [0.3362, 0.038, 1.77211],
    [0.3187, 0.048, 1.7441],
    [0.2908, 0.06, 1.6692],
    [0.2511, 0.0739, 1.5281],
    [0.19536, 0.09098, 1.28764],
    [0.1421, 0.1126, 1.0419],
    [0.09564, 0.13902, 0.8129501],
    [0.05795001, 0.1693, 0.6162],
    [0.03201, 0.20802, 0.46518],
    [0.0147, 0.2586, 0.3533],
    [0.0049, 0.323, 0.272],
    [0.0024, 0.4073, 0.2123],
    [0.0093, 0.503, 0.1582],
    [0.0291, 0.6082, 0.1117],
    [0.06327, 0.71, 0.07824999],
    [0.1096, 0.7932, 0.05725001],
    [0.1655, 0.862, 0.04216],
    [0.2257499, 0.9148501, 0.02984],
    [0.2904, 0.954, 0.0203],
    [0.3597, 0.9803, 0.0134],
    [0.4334499, 0.9949501, 0.008749999],
    [0.5120501, 1.0, 0.005749999],
    [0.5945, 0.995, 0.0039],
    [0.6784, 0.9786, 0.002749999],
    [0.7621, 0.952, 0.0021],
    [0.8425, 0.9154, 0.0018],
    [0.9163, 0.87, 0.001650001],
    [0.9786, 0.8163, 0.0014],
    [1.0263, 0.757, 0.0011],
    [1.0567, 0.6949, 0.001],
    [1.0622, 0.631, 0.0008],
    [1.0456, 0.5668, 0.0006],
    [1.0026, 0.503, 0.00034],
    [0.9384, 0.4412, 0.00024],
    [0.8544499, 0.381, 0.00019],
    [0.7514, 0.321, 0.0001],
    [0.6424, 0.265, 4.999999e-05],
    [0.5419, 0.217, 3e-05],
    [0.4479, 0.175, 2e-05],
    [0.3608, 0.1382, 1e-05],
    [0.2835, 0.107, 0.0],
    [0.2187, 0.0816, 0.0],
    [0.1649, 0.061, 0.0],
    [0.1212, 0.04458, 0.0],
    [0.0874, 0.032, 0.0],
    [0.0636, 0.0232, 0.0],
    [0.04677, 0.017, 0.0],
    [0.0329, 0.01192, 0.0],
    [0.0227, 0.00821, 0.0],
    [0.01584, 0.005723, 0.0],
    [0.01135916, 0.004102, 0.0],
    [0.008110916, 0.002929, 0.0],
    [0.005790346, 0.002091, 0.0],
    [0.004109457, 0.001484, 0.0],
    [0.002899327, 0.001047, 0.0],
    [0.00204919, 0.00074, 0.0],
    [0.001439971, 0.00052, 0.0],
    [0.0009999493, 0.0003611, 0.0],
    [0.0006900786, 0.0002492, 0.0],
    [0.0004760213, 0.0001719, 0.0],
    [0.0003323011, 0.00012, 0.0],
    [0.0002348261, 8.48e-05, 0.0],
    [0.0001661505, 6e-05, 0.0],
    [0.000117413, 4.24e-05, 0.0],
    [8.307527e-05, 3e-05, 0.0],
    [5.870652e-05, 2.12e-05, 0.0],
    [4.150994e-05, 1.499e-05, 0.0],
    [2.935326e-05, 1.06e-05, 0.0],
    [2.067383e-05, 7.4657e-06, 0.0],
    [1.455977e-05, 5.2578e-06, 0.0],
    [1.025398e-05, 3.7029e-06, 0.0],
    [7.221456e-06, 2.6078e-06, 0.0],
    [5.085868e-06, 1.8366e-06, 0.0],
    [3.581652e-06, 1.2934e-06, 0.0],
    [2.522525e-06, 9.1093e-07, 0.0],
    [1.776509e-06, 6.4153e-07, 0.0],
    [1.251141e-06, 4.5181e-07, 0.0],
];
const XYZ_TO_SRGB: [f64; 9] = [3.2406, -1.5372, -0.4986, -0.9689, 1.8758, 0.0415, 0.0557, -0.204, 1.057];

/// Bruneton's `Interpolate` over the 48 wavelengths (exe 0x200554D10).
fn interpolate(values: impl Fn(usize) -> f64, lambda: f64) -> f64 {
    let wavelength = |i: usize| 360.0 + 10.0 * i as f64;
    if lambda < wavelength(0) {
        return values(0);
    }
    for i in 0..47 {
        if lambda < wavelength(i + 1) {
            let u = (lambda - wavelength(i)) / (wavelength(i + 1) - wavelength(i));
            return values(i) * (1.0 - u) + values(i + 1) * u;
        }
    }
    values(47)
}

/// A CIE function at `lambda`, 0 outside (360, 830) (exe 0x200554C70).
fn cie(lambda: f64, column: usize) -> f64 {
    if lambda <= 360.0 || lambda >= 830.0 {
        return 0.0;
    }
    let u = (lambda - 360.0) / 5.0;
    let row = u.floor() as usize;
    let u = u - row as f64;
    CIE[row][column] * (1.0 - u) + CIE[row + 1][column] * u
}

/// The weight of wavelength `lambda` in sRGB channel `channel`, over a
/// band `dlambda` wide (exe 0x200554D80; no 683 lm/W).
fn srgb_weight(lambda: f64, channel: usize, dlambda: f64) -> f32 {
    let m = &XYZ_TO_SRGB[channel * 3..channel * 3 + 3];
    ((m[1] * cie(lambda, 1) + m[0] * cie(lambda, 0) + m[2] * cie(lambda, 2)) * dlambda) as f32
}

/// The wavelengths of spectral pass `i` of 5: 3 per pass over 360..830 nm
/// (exe 0x2005561AA..0x200556325: `(15 + 2) / 3` passes).
fn pass_wavelengths(i: usize) -> [f64; 3] {
    let dlambda = 470.0 / (3.0 * 5.0);
    [0.5, 1.5, 2.5].map(|k| 360.0 + (3.0 * i as f64 + k) * dlambda)
}

const PASSES: usize = 5;
/// Scattering orders per pass (exe 0x200556866 stores 4; 0x200555DD2 loops
/// orders 2 through it).
const ORDERS: u32 = 4;

/// The direction the sun is in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    pub elevation_deg: f32,
    pub azimuth_deg: f32,
}

impl Sun {
    /// The direction sunlight travels (exe 0x20017EA30):
    /// `normalize(cos a cos e, -sin e, sin a cos e)`.
    pub fn light(self) -> [f32; 3] {
        let e = self.elevation_deg / 180.0 * std::f32::consts::PI;
        let a = self.azimuth_deg / 180.0 * std::f32::consts::PI;
        normalize([a.cos() * e.cos(), -e.sin(), a.sin() * e.cos()])
    }
}

/// What 0.69d's atmosphere reads from an `environment` object. Absent
/// fields take the exe's defaults (0x20017BD50).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    pub sun: Sun,
    pub sun_intensity_r2: f32,
    pub sun_disk_radius_scale: f32,
    pub air_density: f32,
    pub air_turbidity: f32,
    pub sky_intensity: f32,
    /// As authored; 0.69d raises it to 2.2 on load (exe 0x2002232A1).
    pub sky_tint: [f32; 3],
    pub fog_tint: [f32; 3],
}

impl Default for Environment {
    fn default() -> Self {
        Environment {
            sun: Sun { elevation_deg: 60.0, azimuth_deg: 0.0 },
            sun_intensity_r2: 1.0,
            sun_disk_radius_scale: 1.0,
            air_density: 1.0,
            air_turbidity: 1.0,
            sky_intensity: 1.0,
            sky_tint: [1.0; 3],
            fog_tint: [1.0; 3],
        }
    }
}

impl Environment {
    pub fn read(environment: &crate::ConfigValue) -> Self {
        use crate::ConfigValue;
        let d = Environment::default();
        let number = |name: &str, default: f32| match environment.named(name) {
            Some(ConfigValue::Float(v)) => *v,
            Some(ConfigValue::Integer(v)) => *v as f32,
            _ => default,
        };
        let colour = |name: &str, default: [f32; 3]| match environment.named(name) {
            Some(ConfigValue::Float4(v)) => [v[0], v[1], v[2]],
            Some(ConfigValue::Float3(v)) => *v,
            _ => default,
        };
        Environment {
            sun: Sun { elevation_deg: number("sun_elevation", d.sun.elevation_deg), azimuth_deg: number("sun_azimuth", d.sun.azimuth_deg) },
            sun_intensity_r2: number("sun_intensity_r2", d.sun_intensity_r2),
            sun_disk_radius_scale: number("sun_disk_radius_scale", d.sun_disk_radius_scale),
            air_density: number("air_density", d.air_density),
            air_turbidity: number("air_turbidity", d.air_turbidity),
            sky_intensity: number("sky_intensity", d.sky_intensity),
            sky_tint: colour("sky_tint", d.sky_tint),
            fog_tint: colour("fog_tint", d.fog_tint),
        }
    }

    /// The precomputation constants of one pass (exe 0x200555E80 from the
    /// parameters 0x2005567C0 builds: Mie `turbidity x 2e-6` /m extinction,
    /// 0.9 of it scattering; lengths in km; the sun's angular radius
    /// `0.004625 x sun_disk_radius_scale`; turbidity and density at least
    /// 0.01, exe 0x200510B31).
    fn pass(&self, lambdas: [f64; 3], to_rgb: [[f32; 3]; 3]) -> PassConstants {
        let km = 1000.0;
        let turbidity = if self.air_turbidity > 0.01 { self.air_turbidity } else { 0.01 } as f64;
        let density = if self.air_density > 0.01 { self.air_density } else { 0.01 };
        let mie_extinction = turbidity * 2e-6;
        let solar = lambdas.map(|l| interpolate(|i| SOLAR_IRRADIANCE[i], l) as f32 * self.sun_intensity_r2);
        let rayleigh = lambdas.map(|l| (interpolate(|i| RAYLEIGH_AT_1NM * (360.0 + 10.0 * i as f64).powf(-4.0), l) * km * density as f64) as f32);
        let ozone = lambdas.map(|l| (interpolate(|i| OZONE_NUMBER_DENSITY * OZONE_CROSS_SECTION[i], l) * km) as f32);
        PassConstants {
            solar: Spectral(solar),
            rayleigh: Spectral(rayleigh),
            sun_angular_radius: (self.sun_disk_radius_scale as f64 * 0.004625) as f32,
            ozone: Spectral(ozone),
            mie_scattering: (mie_extinction * 0.9 * km) as f32,
            mie_extinction: (mie_extinction * km) as f32,
            light: self.sun.light(),
            to_rgb,
        }
    }
}

/// 0.69d's atmosphere for one environment: its tables and what its sky and
/// fog programs compute from them.
pub struct Atmosphere {
    environment: Environment,
    /// At 680, 550, 440 nm (exe 0x20055648E..0x2005564F0), as the consumers
    /// read it.
    transmittance: Table<Spectral>,
    scattering: Table<ScatteringTexel>,
}

impl Atmosphere {
    pub fn new(environment: Environment) -> Self {
        let (w, h) = SCATTERING_SIZE;
        let mut scattering = Table { width: w, height: h, texels: vec![ScatteringTexel::default(); w * h] };
        let add = |a: ScatteringTexel, b: ScatteringTexel| ScatteringTexel {
            rayleigh: Rgb(std::array::from_fn(|k| a.rayleigh.0[k] + b.rayleigh.0[k])),
            mie_red: a.mie_red + b.mie_red,
        };
        for i in 0..PASSES {
            let lambdas = pass_wavelengths(i);
            let dlambda = 470.0 / (3.0 * PASSES as f64);
            let to_rgb = lambdas.map(|l| std::array::from_fn(|k| srgb_weight(l, k, dlambda)));
            let c = environment.pass(lambdas, to_rgb);
            let t = transmittance(&c);
            let mut irradiance = direct_irradiance(&c, &t);
            let (rayleigh, mie, single) = single_scattering(&c, &t);
            scattering = scattering.zip(&single, add);
            let mut multiple: Option<Table<Spectral>> = None;
            for _order in 2..=ORDERS {
                let incident = match &multiple {
                    None => Incident::Single { rayleigh: &rayleigh, mie: &mie },
                    Some(m) => Incident::Multiple(m),
                };
                let density = scattering_density(&c, &t, &incident, &irradiance);
                irradiance = indirect_irradiance(&incident);
                let (delta, added) = multiple_scattering(&c, &density);
                scattering = scattering.zip(&added, add);
                multiple = Some(delta);
            }
        }
        let transmittance = transmittance(&environment.pass([680.0, 550.0, 440.0], [[0.0; 3]; 3]));
        Atmosphere { environment, transmittance, scattering }
    }

    /// The scattering table as 0.69d stores it (`t_scattering`, bound to
    /// `sky_dome.ps` t1 and `sun_lighting.ps` t6): 256 x 64 RGBA, rows by
    /// height, linear sRGB without the 683 lm/W in rgb, single Mie's red
    /// in a; mapping in `scattering_uv`.
    pub fn scattering(&self) -> &Table<ScatteringTexel> {
        &self.scattering
    }

    /// The transmittance table at 680, 550, 440 nm (`t_transmittance`).
    pub fn transmittance(&self) -> &Table<Spectral> {
        &self.transmittance
    }

    /// The light scattered toward an eye at `r` along `dir`, phase functions
    /// applied for the sun: `S.rgb PR + mie PM`, with single Mie's colour
    /// taken from its red and the Rayleigh colour (`sky_dome.ps` and
    /// `sun_lighting.ps`, Rayleigh at density 1: 0.00580450008, 0.0135629,
    /// 0.0331125483 /km).
    fn scattered(&self, r: f32, dir: [f32; 3]) -> Rgb {
        let (u, v) = scattering_uv(r, dir[1], ray_end(r, dir[1]));
        let s = self.scattering.sample(u, v);
        let beta = [0.005_804_5f32, 0.013_562_9, 0.033_112_548].map(|b| b * self.environment.air_density);
        let rgb = s.rayleigh.0;
        let mie: [f32; 3] = if rgb[0] != 0.0 { std::array::from_fn(|k| rgb[k] * s.mie_red / rgb[0] * beta[0] / beta[k]) } else { [0.0; 3] };
        let nu = dot(dir, self.environment.sun.light().map(|v| -v));
        let (pr, pm) = (rayleigh_phase(nu), mie_phase(nu));
        Rgb(std::array::from_fn(|k| rgb[k] * pr + mie[k] * pm))
    }

    /// `sky_dome.ps` (no clouds, stratosphere or sun disc): the sky's
    /// luminance in world direction `dir` for a camera `camera_y` m up,
    /// `683 sky_intensity sky_tint^2.2 (S.rgb PR + mie PM)`.
    pub fn sky(&self, dir: [f32; 3], camera_y: f32) -> Luminance {
        let r = (camera_y * 0.001).max(0.0) + BOTTOM;
        let s = self.scattered(r, normalize(dir));
        let e = &self.environment;
        Luminance(std::array::from_fn(|k| 683.0 * e.sky_intensity * e.sky_tint[k].powf(2.2) * s.0[k]))
    }

    /// `L_h`, the colour `sun_lighting.ps` gives its height fog: the table
    /// toward the horizon at the view's azimuth, just below the true one
    /// (`mu_h |h| / (6360 / r - 0.002)`), so on its ground half, for an eye
    /// `eye_y` m up (at least 1 m), `683 fog_tint^2.2 (S.rgb PR + mie PM)`.
    pub fn horizon(&self, view: [f32; 3], eye_y: f32) -> Luminance {
        let h = (eye_y * 0.001).max(0.001);
        let r = h + BOTTOM;
        let (hx, hz) = if -0.999 < view[1] { (view[0], view[2]) } else { (0.0, 1.0) };
        let mu_h = -rho2(h).sqrt() / r;
        let y = mu_h * (hx * hx + hz * hz).sqrt() / (BOTTOM / r - 0.002);
        let s = self.scattered(r, normalize([hx, y, hz]));
        let e = &self.environment;
        Luminance(std::array::from_fn(|k| 683.0 * e.fog_tint[k].powf(2.2) * s.0[k]))
    }
}

/// The sun's transmittance at the ground as `sun_lighting.ps` reads it
/// (680, 550, 440 nm): the table at an eye 1 m up (its floor) toward the
/// sun, times its horizon smoothstep over the sun's disc.
pub fn sun_transmittance(environment: &Environment) -> [f32; 3] {
    let c = environment.pass([680.0, 550.0, 440.0], [[0.0; 3]; 3]);
    let t = transmittance(&c);
    let r = BOTTOM + 0.001;
    let mu_s = -c.light[1];
    let (u, v) = transmittance_uv(r, mu_s);
    let (s, mu_h, a) = (BOTTOM / r, -rho2(0.001).sqrt() / r, c.sun_angular_radius);
    let f = sat((mu_s - mu_h + s * a) / (2.0 * s * a));
    let f = f * f * (3.0 - 2.0 * f);
    t.sample(u, v).0.map(|v| v * f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f32, want: f32, rel: f32) -> bool {
        (got - want).abs() <= rel * want.abs().max(1e-30)
    }

    /// Rudnya's numbers at 550 nm and one test pass of them.
    fn constants(light: [f32; 3]) -> PassConstants {
        PassConstants {
            solar: Spectral([1.5, 1.8, 1.9]),
            rayleigh: Spectral([0.0116, 0.0271, 0.0662]),
            sun_angular_radius: 0.013_875,
            ozone: Spectral([0.65e-3, 1.881e-3, 0.085e-3]),
            mie_scattering: 0.0162,
            mie_extinction: 0.018,
            light,
            to_rgb: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    const NOON: [f32; 3] = [0.0, -1.0, 0.0];

    #[test]
    fn texel_mappings_read_back_their_texels() {
        // Not the ground half's tangent edge in the lowest rows: there r - 6360
        // loses its digits in f32 (ulp 0.0005 km), on the GPU as here.
        let off: Vec<_> = [(60, 20), (100, 12), (127, 31), (128, 0), (129, 10), (200, 40), (255, 63), (64, 63)]
            .into_iter()
            .map(|(x, y)| {
                let (r, mu, end) = scattering_texel(x, y);
                let (u, v) = scattering_uv(r, mu, end);
                (x, y, u * 256.0 - (x as f32 + 0.5), v * 64.0 - (y as f32 + 0.5))
            })
            .filter(|&(_, _, du, dv)| du.abs() >= 0.05 || dv.abs() >= 0.05)
            .collect();
        assert!(off.is_empty(), "{off:?}");
        // Row 0's ground half is degenerate (rho = h): every reader takes u = 0.498046875.
        assert_eq!(scattering_uv(BOTTOM, -0.5, RayEnd::Ground).0, 0.498_046_88);
        // The ground half is the rays that meet the ground.
        assert_eq!(ray_end(scattering_texel(100, 20).0, scattering_texel(100, 20).1), RayEnd::Ground);
        assert!(close(scattering_texel(128, 0).1, 1.0, 1e-6));
    }

    #[test]
    fn phase_functions_by_hand() {
        // 0.0891267657 + 0.0318309888 = 0.1209577545; at nu = 0, g = 0.76:
        // 0.0195609424 / 1.5776^1.5 = 0.0195609424 / 1.981524 = 0.00987166.
        assert!(close(rayleigh_phase(1.0), 0.120_957_75, 1e-6));
        assert!(close(mie_phase(0.0), 0.009_871_66, 1e-5));
        // Cornette-Shanks integrates to 1 over the sphere.
        let n = 20_000;
        let sum: f32 = (0..n).map(|i| mie_phase(-1.0 + (i as f32 + 0.5) * 2.0 / n as f32) * 2.0 / n as f32).sum();
        assert!(close(sum * 2.0 * std::f32::consts::PI, 1.0, 2e-3), "{sum}");
    }

    #[test]
    fn transmittance_to_the_zenith() {
        // Texel (0, 0) is r = 6360, mu = 1: exp(-(beta_R 8 (1 - e^-7.5) +
        // beta_M 1.2 + beta_O 15)), the integrals of the three profiles.
        let c = constants(NOON);
        let t = transmittance(&c).texel(0, 0);
        for k in 0..3 {
            let tau = c.rayleigh.0[k] * 8.0 * (1.0 - (-7.5f32).exp()) + c.mie_extinction * 1.2 + c.ozone.0[k] * 15.0;
            assert!(close(t.0[k], (-tau).exp(), 1e-4), "{k}: {} {}", t.0[k], (-tau).exp());
        }
        // At the top, looking up: nothing in the way.
        assert!(transmittance(&c).texel(0, 63).0.iter().all(|&v| close(v, 1.0, 1e-5)));
    }

    #[test]
    fn direct_irradiance_by_hand() {
        let c = constants(NOON);
        let t = transmittance(&c);
        let e = direct_irradiance(&c, &t);
        // Texel (63, 0): mu_s = 1 at the ground; reads transmittance texel (0, 0).
        for k in 0..3 {
            assert!(close(e.texel(63, 0).0[k], t.texel(0, 0).0[k] * c.solar.0[k], 1e-5));
        }
        // Texel (31, 0): mu_s = -1/63, below the disc's lower edge -> 0;
        // radius 0.02: (mu_s + 0.02)^2 / 0.08 = 2.1345e-4 of T.
        assert_eq!(e.texel(31, 0).0, [0.0; 3]);
        let wide = PassConstants { sun_angular_radius: 0.02, ..c };
        let got = direct_irradiance(&wide, &t).texel(31, 0).0[1];
        let (u, v) = transmittance_uv(BOTTOM, -1.0 / 63.0);
        let want = (0.02f32 - 1.0 / 63.0).powi(2) / 0.08 * t.sample(u, v).0[1] * c.solar.0[1];
        assert!(close(got, want, 2e-3), "{got} {want}");
    }

    #[test]
    fn single_scattering_toward_a_zenith_sun() {
        // Sun overhead, view up from the ground (texel 128, 0): sun and view
        // transmittance multiply to the whole column's, so Rayleigh single
        // scattering is solar beta_R T(zenith) 8 (1 - e^-7.5), Mie solar
        // beta_M T 1.2 (to the view depth's lag of half a step: 2.5 % in blue).
        let c = constants(NOON);
        let t = transmittance(&c);
        let (r, m) = single_texel(&c, &t, 128, 0);
        for k in 0..3 {
            let tz = t.texel(0, 0).0[k];
            assert!(close(r.0[k], c.solar.0[k] * c.rayleigh.0[k] * tz * 8.0 * (1.0 - (-7.5f32).exp()), 0.02), "{k} {:?}", r);
            assert!(close(m.0[k], c.solar.0[k] * c.mie_scattering * tz * 1.2, 0.03), "{k} {:?}", m);
        }
        // From the top, straight up: a ray of length 0 (to f32 rounding).
        let (top, _) = single_texel(&c, &t, 128, 63);
        assert!(top.0.iter().zip(r.0).all(|(&a, b)| a < b * 1e-5), "{top:?}");
    }

    fn uniform(value: f32) -> Table<Spectral> {
        let (w, h) = SCATTERING_SIZE;
        Table { width: w, height: h, texels: vec![Spectral([value; 3]); w * h] }
    }

    #[test]
    fn density_of_uniform_light_by_hand() {
        // Light L from everywhere, no Mie, a black ground: J = L beta_R
        // rho_R(h) 0.0891267657 Omega (the linear term sums to 0 over the
        // symmetric grid), Omega = 32 x 0.0385531411 x sum sin((m+1/2) pi/16)
        // = 32 x 0.0385531411 / sin(pi/32) = 12.5266.
        let c = PassConstants { mie_scattering: 0.0, ..constants([0.6, -0.8, 0.0]) };
        let t = transmittance(&c);
        let zero = Table { width: 64, height: 16, texels: vec![Spectral::default(); 64 * 16] };
        let l = uniform(2.0);
        let j = scattering_density(&c, &t, &Incident::Multiple(&l), &zero);
        let omega = 32.0 * 0.038_553_14 / (std::f32::consts::PI / 32.0).sin();
        for (x, y) in [(0, 0), (130, 20), (255, 63)] {
            let (r, _, _) = scattering_texel(x, y);
            let rho = (-(r - BOTTOM) / 8.0).exp();
            for k in 0..3 {
                assert!(close(j.texel(x, y).0[k], 2.0 * c.rayleigh.0[k] * rho * 0.089_126_77 * omega, 1e-3), "{x} {y} {k}");
            }
        }
    }

    #[test]
    fn irradiance_of_uniform_light_by_hand() {
        // E = L x 64 x 0.00963828526 x sum sin cos over theta_m = (m + 1/2)
        // pi/32 = L x 64 x 0.00963828526 x 1/2 / sin(pi/32) = 3.14313 L.
        let e = indirect_irradiance(&Incident::Multiple(&uniform(1.0)));
        let want = 64.0 * 0.009_638_285 * 0.5 / (std::f32::consts::PI / 32.0).sin();
        assert!(close(e.texel(10, 3).0[0], want, 1e-4), "{} {want}", e.texel(10, 3).0[0]);
    }

    #[test]
    fn multiple_scattering_of_uniform_density_by_hand() {
        // No extinction: J times the ray's length. Texel (128, 0) looks up
        // 60 km; texel (128, 63) is a ray of length 0.
        let c = PassConstants { rayleigh: Spectral([0.0; 3]), ozone: Spectral([0.0; 3]), mie_extinction: 0.0, ..constants(NOON) };
        let (delta, added) = multiple_scattering(&c, &uniform(0.5));
        assert!(close(delta.texel(128, 0).0[2], 30.0, 1e-5));
        assert!(delta.texel(128, 63).0[2] < 30.0 * 1e-5);
        // Added over the Rayleigh phase of the texel's view. With the sun
        // overhead the programs take the plane (0, 1) and, for |mu| > 0.999,
        // the vector (0, mu, 1): the view of mu = 1 is 45 degrees up.
        assert!(close(added.texel(128, 0).rayleigh.0[2], 30.0 / rayleigh_phase(std::f32::consts::FRAC_1_SQRT_2), 1e-5));
    }

    #[test]
    fn pass_constants_follow_the_exe() {
        // Rayleigh at 550 nm, density 1: 1241090 / 550^4 x 1000 = 0.0135629 /km,
        // the `sky_dome.ps` immediate; Mie 9 x 2e-3 and 9 x 1.8e-3.
        let e = Environment { air_turbidity: 9.0, ..Environment::default() };
        let c = e.pass([680.0, 550.0, 440.0], [[0.0; 3]; 3]);
        assert!(close(c.rayleigh.0[1], 0.013_562_9, 1e-5) && close(c.rayleigh.0[0], 0.005_804_5, 1e-4));
        assert!(close(c.mie_extinction, 0.018, 1e-6) && close(c.mie_scattering, 0.0162, 1e-6));
        assert!(close(c.ozone.0[1], 5.374e18 * 3.5e-25 * 1000.0, 1e-6));
        // The five passes' weights sum to the CIE functions' integral of a
        // flat spectrum: Y = 106.86 nm (sum of y-bar over 5 nm steps).
        let y: f64 = (0..PASSES).flat_map(pass_wavelengths).map(|l| cie(l, 1) * 470.0 / 15.0).sum();
        assert!((y - 106.86).abs() < 1.5, "{y}");
        // Sun at azimuth 0 shines toward +X (it stands toward -X).
        let l = Sun { elevation_deg: 60.0, azimuth_deg: 0.0 }.light();
        assert!(close(l[0], 0.5, 1e-6) && close(l[1], -0.866_025_4, 1e-6) && l[2].abs() < 1e-6);
    }

    fn rudnya() -> Environment {
        Environment {
            sun: Sun { elevation_deg: 60.0, azimuth_deg: 0.0 },
            sun_intensity_r2: 0.85,
            sun_disk_radius_scale: 3.0,
            air_density: 2.0,
            air_turbidity: 9.0,
            sky_intensity: 1.0,
            sky_tint: [1.0; 3],
            fog_tint: [1.0, 0.957, 0.808],
        }
    }

    /// Rudnya's sky and fog colour (`cargo test -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn rudnya_report() {
        let a = Atmosphere::new(rudnya());
        let s = a.scattering();
        let max = s.texels.iter().fold([0.0f32; 4], |m, t| [m[0].max(t.rayleigh.0[0]), m[1].max(t.rayleigh.0[1]), m[2].max(t.rayleigh.0[2]), m[3].max(t.mie_red)]);
        let min = s.texels.iter().fold(f32::MAX, |m, t| m.min(t.rayleigh.0.iter().cloned().fold(t.mie_red, f32::min)));
        println!("scattering table: max rgba {max:?}, min {min}");
        println!("zenith sky {:?} cd/m2", a.sky([0.0, 1.0, 0.0], 20.4).0);
        for el in [0.5f32, 2.0, 5.0, 10.0, 30.0] {
            for az in [0.0f32, 90.0, 180.0] {
                let (e, z) = (el.to_radians(), az.to_radians());
                // az measured from the sun's azimuth (toward -X).
                println!("sky el {el} az {az}: {:?}", a.sky([-e.cos() * z.cos(), e.sin(), -e.cos() * z.sin()], 20.4).0);
            }
        }
        for az in [0.0f32, 45.0, 90.0, 135.0, 180.0] {
            let z = az.to_radians();
            for eye in [2.0f32, 20.0] {
                println!("L_h az {az} eye {eye} m: {:?}", a.horizon([-z.cos(), 0.0, -z.sin()], eye).0);
            }
        }
        println!("sun transmittance {:?}", sun_transmittance(&rudnya()));
    }

    /// Diagnostic: model / probe per sector, total and single scattering only.
    #[test]
    #[ignore]
    fn rudnya_probe_sectors() {
        use super::super::lighting::{cube_direction, decode_069d};
        let path = std::env::var("SKHID_069D_GLOBAL_PROBE").expect("SKHID_069D_GLOBAL_PROBE");
        let cube = crate::dds_cube::decode_cube(&std::fs::read(path).unwrap()).unwrap();
        let env = rudnya();
        let a = Atmosphere::new(env);
        // Single scattering only, all passes.
        let (w, h) = SCATTERING_SIZE;
        let mut single = Table { width: w, height: h, texels: vec![ScatteringTexel::default(); w * h] };
        for i in 0..PASSES {
            let lambdas = pass_wavelengths(i);
            let to_rgb = lambdas.map(|l| std::array::from_fn(|k| srgb_weight(l, k, 470.0 / 15.0)));
            let c = env.pass(lambdas, to_rgb);
            let t = transmittance(&c);
            let (_, _, s) = single_scattering(&c, &t);
            single = single.zip(&s, |a, b| ScatteringTexel { rayleigh: Rgb(std::array::from_fn(|k| a.rayleigh.0[k] + b.rayleigh.0[k])), mie_red: a.mie_red + b.mie_red });
        }
        let b = Atmosphere { environment: env, transmittance: a.transmittance.clone(), scattering: single };
        let exposure = 1.0 / (1.2 * 2f32.powi(12));
        let n = cube.size;
        let mut cells: std::collections::BTreeMap<(i32, i32), Vec<(f32, [f32; 3], [f32; 3], [f32; 3])>> = Default::default();
        for face in 0..6 {
            for y in 0..n {
                for x in 0..n {
                    let d = cube_direction(face, x, y, n);
                    let el = d[1].asin().to_degrees();
                    if el <= 10.0 { continue; }
                    let probe = decode_069d(cube.faces[face][0][y * n + x]);
                    let m = a.sky(d, 20.392).0.map(|v| v * exposure);
                    let s1 = b.sky(d, 20.392).0.map(|v| v * exposure);
                    // azimuth from the sun (toward -X)
                    let az = (-d[2]).atan2(-d[0]).to_degrees().abs();
                    let blue = probe[2] / (probe[0] + probe[1] + probe[2]).max(1e-9);
                    cells.entry(((el / 10.0) as i32, (az / 45.0) as i32)).or_default().push((blue, m, s1, probe));
                }
            }
        }
        println!("el  az-from-sun  n  total/probe rgb  single/probe rgb");
        for ((e, z), mut v) in cells {
            v.sort_by(|p, q| q.0.total_cmp(&p.0));
            let v = &v[..v.len().div_ceil(10)];
            let sum = |f: &dyn Fn(&(f32, [f32; 3], [f32; 3], [f32; 3])) -> [f32; 3]| v.iter().map(f).fold([0.0f32; 3], |a, b| [a[0] + b[0], a[1] + b[1], a[2] + b[2]]);
            let (m, s1, p) = (sum(&|t| t.1), sum(&|t| t.2), sum(&|t| t.3));
            let r = |x: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| (x[k] / p[k] * 100.0).round() / 100.0) };
            println!("{:2} {:3} {:4} {:?} {:?} probe {:?}", e * 10, z * 45, v.len(), r(m), r(s1), p.map(|x| (x / v.len() as f32 * 1000.0).round() / 1000.0));
        }
    }

    /// The check against 0.69d's own sky: Rudnya's `global` probe
    /// (`level_01/day/global_3728.dds`, captured at EV 12 from y 20.392 m,
    /// clouds at one phase, no sun disc), its path in
    /// `SKHID_069D_GLOBAL_PROBE`. Cloud-free texels: the bluest tenth
    /// (b / (r + g + b)) of each 5-degree band and 30-degree sector above 10
    /// degrees. Prints model / probe by band and channel.
    #[test]
    #[ignore]
    fn rudnya_probe_check() {
        use super::super::lighting::{cube_direction, decode_069d};
        let path = std::env::var("SKHID_069D_GLOBAL_PROBE").expect("SKHID_069D_GLOBAL_PROBE");
        let cube = crate::dds_cube::decode_cube(&std::fs::read(path).unwrap()).unwrap();
        let a = Atmosphere::new(rudnya());
        let exposure = 1.0 / (1.2 * 2f32.powi(12));
        // (band, sector) -> (blueness, model, probe)
        let mut cells: std::collections::BTreeMap<(i32, i32), Vec<(f32, [f32; 3], [f32; 3])>> = Default::default();
        let n = cube.size;
        for face in 0..6 {
            for y in 0..n {
                for x in 0..n {
                    let d = cube_direction(face, x, y, n);
                    let el = d[1].asin().to_degrees();
                    if el <= 10.0 {
                        continue;
                    }
                    let probe = decode_069d(cube.faces[face][0][y * n + x]);
                    let model = a.sky(d, 20.392).0.map(|v| v * exposure);
                    let az = d[0].atan2(d[2]).to_degrees();
                    let blue = probe[2] / (probe[0] + probe[1] + probe[2]).max(1e-9);
                    cells.entry(((el / 5.0) as i32, ((az + 180.0) / 30.0) as i32)).or_default().push((blue, model, probe));
                }
            }
        }
        let mut bands: std::collections::BTreeMap<i32, Vec<([f32; 3], [f32; 3])>> = Default::default();
        for ((band, _), mut v) in cells {
            v.sort_by(|p, q| q.0.total_cmp(&p.0));
            let keep = v.len().div_ceil(10);
            bands.entry(band).or_default().extend(v[..keep].iter().map(|&(_, m, p)| (m, p)));
        }
        println!("band   n   model rgb (capture units)      probe rgb               model/probe");
        let mut all = Vec::new();
        for (band, v) in &bands {
            let mean = |f: &dyn Fn(&([f32; 3], [f32; 3])) -> [f32; 3]| {
                let s = v.iter().map(f).fold([0.0f32; 3], |a, b| [a[0] + b[0], a[1] + b[1], a[2] + b[2]]);
                s.map(|x| x / v.len() as f32)
            };
            let (m, p) = (mean(&|t| t.0), mean(&|t| t.1));
            let ratio: [f32; 3] = std::array::from_fn(|k| m[k] / p[k]);
            println!("{:2}-{:2} {:5} {:?} {:?} {:?}", band * 5, band * 5 + 5, v.len(), m, p, ratio);
            all.extend(v.iter().cloned());
        }
        let ratios: Vec<[f32; 3]> = all.iter().map(|(m, p)| std::array::from_fn(|k| m[k] / p[k])).collect();
        for k in 0..3 {
            let mut r: Vec<f32> = ratios.iter().map(|x| x[k]).collect();
            r.sort_by(f32::total_cmp);
            println!("channel {k}: median model/probe {:.3}, quartiles {:.3} {:.3}", r[r.len() / 2], r[r.len() / 4], r[3 * r.len() / 4]);
        }
    }
}
