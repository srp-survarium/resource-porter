// SPDX-License-Identifier: GPL-3.0-or-later
//! Item texts in every language the 0.26g client ships.
//!
//! The game reads `localization/<language>/localization` for the
//! `-localization=` word it was started with: `original` (its default, the
//! developers' Russian), `russian`, `english`, `ukrainian`, `german` and
//! `polish` (the exe's list at 0x9d4f4c; the stock archive and every Skhid
//! patch carry exactly these six). A key missing from a language shows as
//! the key itself, so each new item names its strings in all six.
//!
//! The files hold UTF-8 bytes. [`BinaryConfig`](crate::BinaryConfig) reads a
//! string as Windows-1251, so a text goes in as its UTF-8 bytes read that
//! way ([`game_string`]); written back, they are the same bytes.
use crate::ConfigValue;
use anyhow::{Context, Result, bail};
use encoding_rs::WINDOWS_1251;

/// The client's localization folders.
pub const LANGUAGES: [&str; 6] = [
    "english",
    "original",
    "russian",
    "polish",
    "german",
    "ukrainian",
];

/// One text in the client's languages; `original` is the Russian.
#[derive(Clone, Copy, Debug)]
pub struct Text {
    pub english: &'static str,
    pub russian: &'static str,
    pub ukrainian: &'static str,
    pub german: &'static str,
    pub polish: &'static str,
}

impl Text {
    /// The text for a localization folder.
    pub fn get(&self, language: &str) -> Result<&'static str> {
        Ok(match language {
            "english" => self.english,
            "original" | "russian" => self.russian,
            "ukrainian" => self.ukrainian,
            "german" => self.german,
            "polish" => self.polish,
            _ => bail!("no text for the {language} localization"),
        })
    }
}

/// A text as a binary config holds it: its UTF-8 bytes read as Windows-1251.
pub fn game_string(text: &str) -> String {
    WINDOWS_1251
        .decode_without_bom_handling(text.as_bytes())
        .0
        .into_owned()
}

/// The UTF-8 text a binary config string holds (the inverse of [`game_string`]).
pub fn text_of(game: &str) -> Result<String> {
    let (bytes, _, unmappable) = WINDOWS_1251.encode(game);
    if unmappable {
        bail!("not a game string");
    }
    Ok(String::from_utf8(bytes.into_owned())?)
}

/// Sets `key` in a localization's `strings` to `text` (in that language),
/// adding it or replacing the text an earlier build gave it.
pub fn set_string(root: &mut ConfigValue, key: &str, text: &str) -> Result<()> {
    let Some(ConfigValue::Named(strings)) = root.named_mut("strings") else {
        bail!("localization has no strings table");
    };
    let value = ConfigValue::String(game_string(text));
    match strings.iter_mut().find(|entry| entry.name == key) {
        Some(entry) => entry.value = value,
        None => strings.push(crate::NamedValue {
            name: key.to_owned(),
            value,
        }),
    }
    Ok(())
}

/// The text of `key` in a localization, if it has one.
pub fn string_of(root: &ConfigValue, key: &str) -> Result<Option<String>> {
    match root.named("strings").and_then(|strings| strings.named(key)) {
        Some(ConfigValue::String(text)) => Ok(Some(text_of(text)?)),
        Some(_) => bail!("{key} is not a string"),
        None => Ok(None),
    }
}

/// Every key an item config shows (its name and description) has a text in
/// each localization: `localizations` are `(language, root)`.
pub fn check_item_texts(item: &ConfigValue, localizations: &[(&str, &ConfigValue)]) -> Result<()> {
    let descriptions = item
        .named("ui_desc")
        .and_then(|ui| ui.named("text_descriptions"))
        .context("item has no ui_desc.text_descriptions")?;
    for field in ["name", "description"] {
        let Some(ConfigValue::String(key)) = descriptions.named(field) else {
            bail!("item has no {field} key");
        };
        for language in LANGUAGES {
            let root = localizations
                .iter()
                .find(|(l, _)| *l == language)
                .with_context(|| format!("no {language} localization"))?
                .1;
            match string_of(root, key)? {
                Some(text) if !text.trim().is_empty() => {}
                _ => bail!("{key} has no {language} text"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;

    #[test]
    fn texts_round_trip_as_the_game_stores_them() {
        // "И" is D0 98 in UTF-8; 0x98 has no Windows-1251 letter but
        // round-trips, as in the stock "ОПИСАНИЕ".
        for text in [
            "Экзоскелет \"Долга\"",
            "Шолом екзоскелета \"Обов'язку\"",
            "Hełm",
            "Exoskelett ß",
            "ИИ",
        ] {
            let game = game_string(text);
            let (bytes, _, bad) = WINDOWS_1251.encode(&game);
            assert!(!bad);
            assert_eq!(&*bytes, text.as_bytes());
            assert_eq!(text_of(&game).unwrap(), text);
        }
    }

    fn localization() -> ConfigValue {
        ConfigValue::Named(vec![NamedValue {
            name: "strings".into(),
            value: ConfigValue::Named(vec![NamedValue {
                name: "st_old".into(),
                value: ConfigValue::String("Old".into()),
            }]),
        }])
    }

    #[test]
    fn a_string_is_added_or_its_text_replaced() {
        let mut root = localization();
        set_string(&mut root, "st_old", "Старый").unwrap();
        set_string(&mut root, "st_new", "Новый").unwrap();
        assert_eq!(
            string_of(&root, "st_old").unwrap().as_deref(),
            Some("Старый")
        );
        assert_eq!(
            string_of(&root, "st_new").unwrap().as_deref(),
            Some("Новый")
        );
        assert_eq!(string_of(&root, "st_none").unwrap(), None);
    }

    #[test]
    fn an_item_needs_its_texts_in_every_language() {
        let item = ConfigValue::Named(vec![NamedValue {
            name: "ui_desc".into(),
            value: ConfigValue::Named(vec![NamedValue {
                name: "text_descriptions".into(),
                value: ConfigValue::Named(vec![
                    NamedValue {
                        name: "name".into(),
                        value: ConfigValue::String("st_n".into()),
                    },
                    NamedValue {
                        name: "description".into(),
                        value: ConfigValue::String("st_d".into()),
                    },
                ]),
            }]),
        }]);
        let mut roots: Vec<ConfigValue> = LANGUAGES.iter().map(|_| localization()).collect();
        for root in &mut roots {
            set_string(root, "st_n", "N").unwrap();
            set_string(root, "st_d", "D").unwrap();
        }
        let view = |roots: &[ConfigValue]| -> Vec<(&str, ConfigValue)> {
            LANGUAGES
                .iter()
                .copied()
                .zip(roots.iter().cloned())
                .collect()
        };
        let all = view(&roots);
        let refs: Vec<(&str, &ConfigValue)> = all.iter().map(|(l, r)| (*l, r)).collect();
        check_item_texts(&item, &refs).unwrap();
        // Polish without the description: refused.
        roots[3] = localization();
        set_string(&mut roots[3], "st_n", "N").unwrap();
        let all = view(&roots);
        let refs: Vec<(&str, &ConfigValue)> = all.iter().map(|(l, r)| (*l, r)).collect();
        let error = check_item_texts(&item, &refs).unwrap_err().to_string();
        assert!(error.contains("polish"), "{error}");
        // A language missing altogether: refused.
        assert!(check_item_texts(&item, &refs[..5]).is_err());
    }
}
