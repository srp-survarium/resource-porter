use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use vostok_resource_porter::dirty_bomb::package::Package;

fn main() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let root = PathBuf::from(
        arguments
            .next()
            .context("usage: scan_dirty_bomb_exports ROOT NEEDLE...")?,
    );
    let arguments = arguments
        .map(|argument| argument.to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let dump_properties = arguments.iter().any(|argument| argument == "--properties");
    let needles = arguments
        .into_iter()
        .filter(|argument| argument != "--properties")
        .collect::<Vec<_>>();
    if needles.is_empty() {
        bail!("usage: scan_dirty_bomb_exports ROOT NEEDLE...");
    }
    let mut packages = Vec::new();
    if root.is_file() {
        packages.push(root);
    } else {
        collect_packages(&root, &mut packages)?;
    }
    packages.sort();
    for path in packages {
        let package = match Package::read(&path) {
            Ok(package) => package,
            Err(error) => {
                eprintln!("skip={} error={error:#}", path.display());
                continue;
            }
        };
        for index in 1..=package.exports.len() {
            let object = package.object_path(i32::try_from(index)?)?;
            let lower = object.to_ascii_lowercase();
            if needles.iter().any(|needle| lower.contains(needle)) {
                let export = &package.exports[index - 1];
                println!(
                    "package={} class={} object={}",
                    path.display(),
                    export.class_name,
                    object
                );
                if dump_properties {
                    for property in package.properties(export)? {
                        let payload = property
                            .payload
                            .iter()
                            .take(16)
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>();
                        println!(
                            "  property={} type={} bool={:?} bytes={} payload={}",
                            property.name,
                            property.type_name,
                            property.bool_value,
                            property.payload.len(),
                            payload,
                        );
                        if property.name == "StichedActorsNames" {
                            if let Some(names) = ascii_fstring_array(property.payload) {
                                println!("    strings={names:?}");
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn ascii_fstring_array(mut bytes: &[u8]) -> Option<Vec<String>> {
    let take_i32 = |bytes: &mut &[u8]| {
        let value = i32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
        *bytes = bytes.get(4..)?;
        Some(value)
    };
    let count = usize::try_from(take_i32(&mut bytes)?).ok()?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let length = usize::try_from(take_i32(&mut bytes)?).ok()?;
        let raw = bytes.get(..length)?;
        bytes = bytes.get(length..)?;
        let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
        values.push(String::from_utf8(raw.to_vec()).ok()?);
    }
    Some(values)
}

fn collect_packages(directory: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("listing package directory {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_packages(&path, output)?;
        } else if path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("umap") || extension.eq_ignore_ascii_case("upk")
        }) {
            output.push(path);
        }
    }
    Ok(())
}
