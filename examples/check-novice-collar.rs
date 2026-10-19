//! The native neck under the Novice collar and under stock collars, through
//! the lobby's and the match's clips.
use std::path::PathBuf;
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let db = PathBuf::from(args.next().expect("DB"));
    let bundle = PathBuf::from(args.next().expect("BUNDLE"));
    let patches: Vec<PathBuf> = args.map(PathBuf::from).collect();
    let mut stock = vostok_resource_porter::ResourceDb::open(&db)?;
    let mut jackets = vec![];
    let root = bundle.join("overlay/models/ported/novice_026/torso.skinned_model/render");
    jackets.push((
        "Novice jacket".to_owned(),
        std::fs::read(root.join("torso/converted_model"))?,
        std::fs::read(root.join("bind_pose"))?,
    ));
    for (label, model, surface) in [
        ("Quilted jacket", "scavengers/scavengers_01_torso_s1", "scavengers_01_torso_s"),
        ("Stalker's jacket", "edge/edge_stalker_02_torso_s1", "edge_stalker_02_torso_s"),
        ("Thug's jacket", "black_market/black_market_02_torso_s1", "black_market_02_torso_s"),
    ] {
        let r = format!("models/character/human/{model}.skinned_model/render");
        jackets.push((
            label.to_owned(),
            stock.read(&format!("{r}/{surface}/converted_model"))?,
            stock.read(&format!("{r}/bind_pose"))?,
        ));
    }
    vostok_resource_porter::novice_026::collar_poses(&db, &patches, &jackets)
}
