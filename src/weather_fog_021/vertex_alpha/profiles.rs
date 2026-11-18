//! Active original painted-alpha materials encode both visible and shadow cutoffs.
use anyhow::{Context, Result};

/// Only combinations present in the 28 alpha-tested original materials are valid.
/// The original depth pass uses an ordinary threshold, not the painted-alpha mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthoredProfile {
    MossHalf,
    StuccoQuarter,
    DirtHalf,
    RootsQuarter,
    DirtQuarter,
}

impl AuthoredProfile {
    pub const ALL: [Self; 5] = [
        Self::MossHalf,
        Self::StuccoQuarter,
        Self::DirtHalf,
        Self::RootsQuarter,
        Self::DirtQuarter,
    ];

    pub fn from_authored(dark: f32, light: f32, multiply: bool, threshold: f32) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|p| {
                p.mask_parameters() == (dark, light, multiply) && p.shadow_threshold() == threshold
            })
            .context("unrepresented active original vertex-alpha/shadow profile")
    }

    pub fn marker(self) -> f32 {
        match self {
            Self::MossHalf => -21210.0,
            Self::StuccoQuarter => -21211.0,
            Self::DirtHalf => -21212.0,
            Self::RootsQuarter => -21213.0,
            // -21214 belonged to an inactive material in the initial diagnostic.
            // It is deliberately not a production selector.
            Self::DirtQuarter => -21215.0,
        }
    }

    pub fn mask_parameters(self) -> (f32, f32, bool) {
        match self {
            Self::MossHalf => (0.1, 0.5, false),
            Self::StuccoQuarter => (0.5, 1.0, false),
            Self::DirtHalf | Self::DirtQuarter => (1.0, 1.0, false),
            Self::RootsQuarter => (1.0, 1.0, true),
        }
    }

    pub fn shadow_threshold(self) -> f32 {
        match self {
            Self::MossHalf | Self::DirtHalf => 0.5,
            Self::StuccoQuarter | Self::RootsQuarter | Self::DirtQuarter => 0.25,
        }
    }
}
