use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortManifest {
    pub format_version: u32,
    pub asset: AssetSpec,
    pub weapon: WeaponSpec,
    pub ammo: AmmoSpec,
    pub dictionary: DictionarySpec,
    pub localization: LocalizationSpec,
    #[serde(default)]
    pub copy: CopySpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSpec {
    pub id: String,
    pub donor_build: String,
    pub target_build: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponSpec {
    pub dict_id: u32,
    pub item_category: i32,
    pub donor_config: String,
    pub template_config: String,
    pub target_config: String,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmmoSpec {
    pub dict_id: u32,
    pub item_category: i32,
    pub donor_config: String,
    pub template_config: String,
    pub target_config: String,
    /// Reuse an ammunition item already present in the target database.
    #[serde(default)]
    pub reuse_existing: bool,
    #[serde(default)]
    pub patch: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictionarySpec {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizationSpec {
    pub path: String,
    pub strings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopySpec {
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuildRecord {
    pub format_version: u32,
    pub asset_id: String,
    pub donor_build: String,
    pub target_build: String,
    pub weapon_dict_id: u32,
    pub ammo_dict_id: u32,
    pub files: Vec<BuiltFile>,
    pub shared_dependencies: Vec<String>,
    pub unresolved_references: Vec<String>,
}

/// Runtime catalog additions consumed by SRP.
///
/// This deliberately contains only additive IDs. Existing demo content stays
/// owned by SRP, so importing a weapon cannot silently replace an old one.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SrpContentManifest {
    pub format_version: u32,
    pub extra_weapon_dict_ids: Vec<u16>,
    pub extra_ammo_dict_ids: Vec<u16>,
    pub primary_weapon_dict_id: Option<u16>,
    pub primary_ammo_dict_id: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuiltFile {
    pub path: String,
    pub source: String,
    pub size: u64,
    pub sha256: String,
}

impl PortManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading port manifest {}", path.display()))?;
        let manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing port manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!(
                "unsupported manifest format {}, expected 1",
                self.format_version
            );
        }
        if self.asset.id.is_empty() {
            bail!("asset.id must not be empty");
        }
        if self.weapon.dict_id == self.ammo.dict_id {
            bail!("weapon and ammunition must have distinct dictionary IDs");
        }
        if self.weapon.target_config == self.ammo.target_config {
            bail!("weapon and ammunition target paths must differ");
        }
        Ok(())
    }
}
