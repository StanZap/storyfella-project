//! Immutable originals shared by every Atelier project. Projects retain their
//! own embedded copy, so archiving a library item cannot break a saved project.
use crate::document::{self, Reference};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub fn id(reference: &Reference) -> String {
    if !reference.fingerprint.is_empty() {
        return reference.fingerprint.clone();
    }
    fingerprint(&reference.image)
}
pub fn fingerprint(image: &str) -> String {
    Sha256::digest(image.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
#[derive(Serialize, Deserialize)]
struct Record {
    name: String,
    favorite: bool,
    tags: String,
    provenance: String,
    parent: Option<String>,
    prompt: String,
}
impl Record {
    fn of(reference: &Reference) -> Self {
        Self {
            name: reference.name.clone(),
            favorite: reference.favorite,
            tags: reference.tags.clone(),
            provenance: reference.provenance.clone(),
            parent: reference.parent.clone(),
            prompt: reference.prompt.clone(),
        }
    }
}

fn valid_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid library image identifier"
    );
    Ok(())
}

pub fn store(directory: &Path, reference: &Reference) -> Result<()> {
    document::decode_reference(reference)?;
    if let Some(parent) = &reference.parent {
        valid_id(parent)?;
    }
    fs::create_dir_all(directory)?;
    let id = id(reference);
    let path = directory.join(format!("{id}.json"));
    if path.exists() {
        // Reusing an original must not replace its established edit lineage or
        // library metadata with a project's local name/tags.
        return Ok(());
    }
    let bytes = STANDARD.decode(&reference.image)?;
    document::atomic_write(&directory.join(format!("{id}.image")), &bytes)?;
    document::atomic_write(&path, &serde_json::to_vec(&Record::of(reference))?)
}
pub enum Update {
    Details,
    Favorite,
}
pub fn update(directory: &Path, reference: &Reference, update: Update) -> Result<()> {
    let path = directory.join(format!("{}.json", id(reference)));
    ensure!(path.exists(), "Image is not saved in the library");
    let mut record: Record = serde_json::from_slice(&fs::read(&path)?)?;
    match update {
        Update::Details => {
            ensure!(
                !reference.name.trim().is_empty()
                    && reference.name.len() <= 512
                    && reference.tags.len() <= 32_768,
                "Invalid library name or tags"
            );
            record.name = reference.name.clone();
            record.tags = reference.tags.clone();
        }
        Update::Favorite => record.favorite = reference.favorite,
    }
    document::atomic_write(&path, &serde_json::to_vec(&record)?)
}

pub fn load(directory: &Path) -> Result<Vec<Reference>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut images = Vec::new();
    let mut total = 0usize;
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        ensure!(
            fs::metadata(&path)?.len() <= 28 * 1024 * 1024,
            "Library record exceeds 28 MB"
        );
        let record: Record = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("Could not read library image {}", path.display()))?;
        let identifier = path
            .file_stem()
            .context("Missing image identifier")?
            .to_string_lossy();
        valid_id(&identifier)?;
        let original = directory.join(format!("{identifier}.image"));
        ensure!(
            fs::metadata(&original)?.len() <= 20 * 1024 * 1024,
            "Library original exceeds 20 MB"
        );
        let image = STANDARD.encode(fs::read(original)?);
        let reference = Reference {
            fingerprint: fingerprint(&image),
            name: record.name,
            image,
            favorite: record.favorite,
            tags: record.tags,
            provenance: record.provenance,
            parent: record.parent,
            prompt: record.prompt,
        };
        ensure!(
            path.file_stem()
                .is_some_and(|s| s == id(&reference).as_str()),
            "Library image identifier does not match its original"
        );
        document::decode_reference(&reference)?;
        total += reference.image.len();
        ensure!(
            total <= 512 * 1024 * 1024,
            "Library exceeds the 512 MB in-memory limit"
        );
        images.push(reference);
    }
    Ok(images)
}
pub fn family(
    reference: &Reference,
    images: &[std::sync::Arc<Reference>],
) -> std::collections::HashSet<String> {
    fn root(reference: &Reference, images: &[std::sync::Arc<Reference>]) -> String {
        let mut current = reference;
        let mut visited = std::collections::HashSet::new();
        while let Some(parent) = &current.parent {
            if !visited.insert(id(current)) {
                break;
            }
            let Some(next) = images.iter().find(|r| id(r) == *parent) else {
                return parent.clone();
            };
            current = next;
        }
        id(current)
    }
    let root_id = root(reference, images);
    images
        .iter()
        .filter(|r| root(r, images) == root_id)
        .map(|r| id(r))
        .collect()
}
