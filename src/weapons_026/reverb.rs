// SPDX-License-Identifier: GPL-3.0-or-later
//! A firing sound's reverb layer, as 0.26g can play it.
//!
//! The later builds' firing composites name a reverb composite,
//! `weapons/environments/weapons_<view>_view*`, which holds an item per
//! location (`sound_environment`: `rudnya_outdoor`, `school_class`, ...); the
//! game plays the one of the shooter's sound zone. 0.26g has neither: its
//! composite cooker reads no `sound_environment` and its maps have no zones.
//! A port plays the Rudnya outdoor item ([`OUTDOOR`]) on every shot, patch
//! 19's choice.
//!
//! That item is a collection (the first view's tail, which the shooter
//! hears) or, in the third view, a composite of its own: the near tail (0.31
//! and later: to 21 m) and the distant shot (`main_distant_outdoor` to 200
//! m, `*_outdoor_core_01` and `rudnya_outdoor_distant_*` to 240 m; the
//! one-shot guns' five distant layers). No 0.26g composite nests a
//! composite (none of the 128 stock ones does), so its items go into the
//! gun's composite ([`reverb_layers`]): each one's window is the gun item's
//! and the outdoor item's intersected with its own, the start offsets add
//! up and it plays only where all three are enabled. Patch 19 kept only the
//! near tail (at 0-2000 m): other players' shots fell silent at 20-50 m.
//!
//! A nested layer then ends where its loudness curve does ([`end_at_reach`]):
//! the `volume` curve of its sound's SPL preset, x in metres, reaches 0 at
//! its last key (`main_distant` 200 m, `core_distant` 240 m,
//! `main_3rd_view` 19.3 m), so past it the layer would hold a voice of the
//! sound pool for nothing.

use anyhow::{Context, Result, bail, ensure};

use super::{s, string_of};
use crate::ConfigValue as V;

/// The location every ported shot plays in.
pub const OUTDOOR: &str = "rudnya_outdoor";

/// Whether a firing composite's item names a reverb composite.
pub fn is_reverb(name: &str) -> bool {
    name.starts_with("weapons/environments/weapons_")
}

/// The file of a composite sound.
pub fn composite_file(name: &str) -> String {
    format!("sounds/composite/{name}.composite_sound")
}

/// A sound item's resource type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Single,
    Composite,
    Collection,
}

impl Kind {
    fn parse(v: Option<&V>) -> Result<Self> {
        Ok(match v {
            None | Some(V::Integer(256)) => Self::Single,
            Some(V::Integer(257)) => Self::Composite,
            Some(V::Integer(258)) => Self::Collection,
            other => bail!("sound resource type {other:?}"),
        })
    }

    fn code(self) -> i32 {
        match self {
            Self::Single => 256,
            Self::Composite => 257,
            Self::Collection => 258,
        }
    }
}

/// When and where a composite's item plays: milliseconds after the
/// composite starts, the listener's distance in metres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Play {
    pub start_offset: i32,
    pub start_offset_max: i32,
    pub min_distance: i32,
    pub max_distance: i32,
    pub enabled: bool,
}

impl Play {
    /// An item of a composite played as an item of another one; `None`
    /// where the two windows do not meet.
    pub fn nest(self, inner: Play) -> Option<Play> {
        let play = Play {
            start_offset: self.start_offset + inner.start_offset,
            start_offset_max: self.start_offset_max + inner.start_offset_max,
            min_distance: self.min_distance.max(inner.min_distance),
            max_distance: self.max_distance.min(inner.max_distance),
            enabled: self.enabled && inner.enabled,
        };
        (play.min_distance < play.max_distance).then_some(play)
    }
}

/// A composite's item: its sound and how it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub play: Play,
}

fn int(item: &V, key: &str) -> Result<i32> {
    match item.named(key) {
        Some(V::Integer(v)) => Ok(*v),
        None => Ok(0),
        other => bail!("{key}: {other:?}"),
    }
}

impl Item {
    pub fn parse(item: &V) -> Result<Self> {
        let name = string_of(item.named("filename"))
            .context("a sound item without a file")?
            .to_owned();
        let max_distance = match item.named("max_distance") {
            Some(V::Integer(v)) => *v,
            other => bail!("{name}: max_distance {other:?}"),
        };
        Ok(Self {
            kind: Kind::parse(item.named("resource_type")).context(name.clone())?,
            play: Play {
                start_offset: int(item, "start_offset").context(name.clone())?,
                start_offset_max: int(item, "start_offset_max").context(name.clone())?,
                min_distance: int(item, "min_distance").context(name.clone())?,
                max_distance,
                enabled: !matches!(item.named("enabled"), Some(V::Boolean(false))),
            },
            name,
        })
    }

    /// The item on `template`'s keys (a 0.26g composite's item).
    pub fn to_config(&self, template: &V) -> Result<V> {
        let mut v = template.clone();
        v.set_path("filename", s(self.name.clone()))?;
        v.set_path("resource_type", V::Integer(self.kind.code()))?;
        v.set_path("start_offset", V::Integer(self.play.start_offset))?;
        v.set_path("start_offset_max", V::Integer(self.play.start_offset_max))?;
        v.set_path("min_distance", V::Integer(self.play.min_distance))?;
        v.set_path("max_distance", V::Integer(self.play.max_distance))?;
        v.set_path("enabled", V::Boolean(self.play.enabled))?;
        Ok(v)
    }
}

/// A composite's items.
pub fn items(composite: &V) -> Result<&[V]> {
    match composite
        .named("composite_sound")
        .and_then(|c| c.named("sound_items"))
    {
        Some(V::Indexed(items)) => Ok(items),
        _ => bail!("not a composite sound"),
    }
}

/// What a firing composite's reverb item plays in 0.26g.
#[derive(Debug, PartialEq)]
pub enum Reverb {
    /// The outdoor item is a collection (a first view's tail): the gun's
    /// item names it.
    Collection(Item),
    /// The outdoor item is a composite (the third view's near tail and
    /// distant shot): its items, nested into the gun's composite.
    Nested(Vec<Item>),
}

impl Reverb {
    pub fn layers(&self) -> &[Item] {
        match self {
            Self::Collection(item) => std::slice::from_ref(item),
            Self::Nested(items) => items,
        }
    }
}

/// The layers the gun's item `outer`, which names the reverb composite
/// `reverb`, plays in 0.26g: its Rudnya outdoor item, a nested composite's
/// items (read with `config`, by path) put into the gun's composite.
pub fn reverb_layers(
    outer: &Item,
    reverb: &V,
    config: &mut dyn FnMut(&str) -> Result<Option<V>>,
) -> Result<Reverb> {
    let mut outdoor = items(reverb)?
        .iter()
        .filter(|i| string_of(i.named("sound_environment")) == Some(OUTDOOR));
    let (Some(found), None) = (outdoor.next(), outdoor.next()) else {
        bail!("{}: not one {OUTDOOR} item", outer.name);
    };
    let found = Item::parse(found)?;
    let Some(play) = outer.play.nest(found.play) else {
        bail!("{}: its {OUTDOOR} item never plays", outer.name);
    };
    if found.kind != Kind::Composite {
        return Ok(Reverb::Collection(Item { play, ..found }));
    }
    let nested = required(config, &composite_file(&found.name))?;
    let mut layers = vec![];
    for item in items(&nested)? {
        let item = Item::parse(item)?;
        ensure!(
            item.kind != Kind::Composite,
            "{}: a composite three deep ({})",
            found.name,
            item.name
        );
        if let Some(p) = play.nest(item.play) {
            layers.push(Item { play: p, ..item });
        }
    }
    ensure!(!layers.is_empty(), "{}: no layer plays", found.name);
    Ok(Reverb::Nested(layers))
}

/// Ends each nested layer where its sound falls silent: `reach` answers the
/// metres past which a sound is silent (`None`: unknown, or never).
pub fn end_at_reach(
    reverb: &mut Reverb,
    reach: &mut dyn FnMut(Kind, &str) -> Result<Option<f32>>,
) -> Result<()> {
    let Reverb::Nested(layers) = reverb else {
        return Ok(());
    };
    for layer in layers {
        if let Some(m) = reach(layer.kind, &layer.name)? {
            let end = m.ceil() as i32;
            if end > layer.play.min_distance {
                layer.play.max_distance = layer.play.max_distance.min(end);
            }
        }
    }
    Ok(())
}

/// The metres past which an SPL preset's `volume` curve is silent: its last
/// key's x, when that key's volume is 0 (`None` otherwise).
pub fn silent_past(spl: &V) -> Result<Option<f32>> {
    let Some(V::Named(keys)) = spl.named("volume") else {
        bail!("an SPL preset without a volume curve");
    };
    let mut last: Option<[f32; 2]> = None;
    for k in keys {
        let Some(V::Float2(p)) = k.value.named("position") else {
            bail!("volume.{}: no position", k.name);
        };
        if last.is_none_or(|l| p[0] > l[0]) {
            last = Some(*p);
        }
    }
    Ok(last.filter(|l| l[1] <= 0.0).map(|l| l[0]))
}

/// The sounds of a collection.
pub fn collection_sounds(collection: &V) -> Result<Vec<(Kind, String)>> {
    match collection
        .named("collection")
        .and_then(|c| c.named("sound_items"))
    {
        Some(V::Indexed(items)) => items
            .iter()
            .map(|i| {
                let name =
                    string_of(i.named("filename")).context("a collection item without a file")?;
                Ok((Kind::parse(i.named("resource_type"))?, name.to_owned()))
            })
            .collect(),
        _ => bail!("not a sound collection"),
    }
}

/// How far a sound is heard: its SPL preset's silent point, the farthest of
/// a collection's sounds; `None` for a sound without a preset of a known
/// end (or a composite). `config` reads a config by path (`None`: no file).
pub fn reach(
    kind: Kind,
    name: &str,
    config: &mut dyn FnMut(&str) -> Result<Option<V>>,
) -> Result<Option<f32>> {
    match kind {
        Kind::Composite => Ok(None),
        Kind::Single => {
            let Some(options) = config(&format!("sounds/single/{name}.raw_options"))? else {
                return Ok(None);
            };
            // No preset or `default`: 0.26g skips the curve and plays at
            // 2/d of full volume past 2 m (exe 0x5c9b5), never silent.
            let Some(preset) = super::resources::spl_preset(&options) else {
                return Ok(None);
            };
            if preset == "default" {
                return Ok(None);
            }
            let path = super::resources::spl_file(preset);
            let spl = config(&path)?.with_context(|| format!("{name}: no {path}"))?;
            silent_past(&spl).with_context(|| path.clone())
        }
        Kind::Collection => {
            let path = format!("sounds/collections/{name}.sound_collection");
            let collection = config(&path)?.with_context(|| format!("no {path}"))?;
            let mut far = 0f32;
            for (kind, name) in collection_sounds(&collection).context(path.clone())? {
                match reach(kind, &name, config)? {
                    Some(m) => far = far.max(m),
                    None => return Ok(None),
                }
            }
            Ok(Some(far))
        }
    }
}

/// The layers a firing composite's reverb item plays, each nested one
/// ending at its reach. `config` reads a config by path (`None`: no file):
/// the reverb composites and the sounds' options and SPL presets.
/// `composite` is the firing composite `item` is in.
pub fn layers_of(
    composite: &V,
    item: &Item,
    config: &mut dyn FnMut(&str) -> Result<Option<V>>,
) -> Result<Reverb> {
    let root = required(config, &composite_file(&item.name))?;
    let mut layers = reverb_layers(item, &root, config)?;
    end_at_reach(&mut layers, &mut |kind, name| reach(kind, name, config))?;
    if let Reverb::Nested(_) = layers {
        no_master_layer(composite)?;
    }
    Ok(layers)
}

/// Under a master layer every layer takes the master's volume (exe
/// 0x63d60): a nested layer's own curve would not apply, nor would the
/// master's index hold once layers are put in. (The first view's
/// composites have one; their reverb stays one collection.)
fn no_master_layer(composite: &V) -> Result<()> {
    let master = composite
        .named("composite_sound")
        .and_then(|c| c.named("use_master_layer_index"));
    ensure!(
        !matches!(master, Some(V::Boolean(true))),
        "a reverb layer in a composite with a master layer"
    );
    Ok(())
}

fn with_items(composite: &V, items: Vec<V>) -> Result<V> {
    let mut root = composite.clone();
    root.set_path("composite_sound.sound_items", V::Indexed(items))?;
    Ok(root)
}

/// A donor's firing composite with every reverb item's layers in its place
/// (unchanged when it names no reverb composite).
pub fn with_reverb_layers(
    composite: &V,
    config: &mut dyn FnMut(&str) -> Result<Option<V>>,
) -> Result<V> {
    let mut out = vec![];
    for raw in items(composite)? {
        let item = Item::parse(raw)?;
        if !is_reverb(&item.name) {
            out.push(raw.clone());
            continue;
        }
        for layer in layers_of(composite, &item, config)?.layers() {
            out.push(layer.to_config(raw)?);
        }
    }
    with_items(composite, out)
}

/// A firing composite an earlier port made from `donor` (each reverb item
/// one outdoor collection, patch 19's way) with the donor's reverb layers in
/// place of that collection; its other items stay as they are.
pub fn relayer(
    live: &V,
    donor: &V,
    config: &mut dyn FnMut(&str) -> Result<Option<V>>,
) -> Result<V> {
    let (ours, theirs) = (items(live)?, items(donor)?);
    ensure!(
        ours.len() == theirs.len(),
        "{} items, the donor's {}",
        ours.len(),
        theirs.len()
    );
    let mut out = vec![];
    for (raw, donor_raw) in ours.iter().zip(theirs) {
        let item = Item::parse(donor_raw)?;
        if !is_reverb(&item.name) {
            out.push(raw.clone());
            continue;
        }
        let was = Item::parse(raw)?;
        ensure!(
            was.play == item.play,
            "{}: the port's {:?}, the donor's {:?}",
            was.name,
            was.play,
            item.play
        );
        let layers = layers_of(live, &item, config)?;
        ensure!(
            layers.layers().iter().any(|l| l.name == was.name),
            "{}: none of {}'s layers",
            was.name,
            item.name
        );
        for layer in layers.layers() {
            out.push(layer.to_config(raw)?);
        }
    }
    with_items(live, out)
}

fn required(config: &mut dyn FnMut(&str) -> Result<Option<V>>, path: &str) -> Result<V> {
    config(path)?.with_context(|| format!("no {path}"))
}

/// A firing composite an earlier port made (patch 19's way) and its donor:
/// a row of `ports/weapons-reverb-layers.tsv`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relayer {
    /// The players' composite, by name (`weapons/fn_p90_m`).
    pub composite: String,
    /// The donor build (a key of the builds given to `relayer-weapons`).
    pub build: String,
    /// The donor's composite it was made from.
    pub donor_composite: String,
}

/// The rows of a relayer list: `<composite>\t<build>\t<donor composite>`,
/// `#` comments.
pub fn load_relayers(text: &str) -> Result<Vec<Relayer>> {
    let mut rows = vec![];
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let [composite, build, donor_composite] = line.split('\t').collect::<Vec<_>>()[..] else {
            bail!("line {}: not three columns", n + 1);
        };
        ensure!(
            !rows.iter().any(|r: &Relayer| r.composite == composite),
            "line {}: {composite} twice",
            n + 1
        );
        rows.push(Relayer {
            composite: composite.into(),
            build: build.into(),
            donor_composite: donor_composite.into(),
        });
    }
    Ok(rows)
}
