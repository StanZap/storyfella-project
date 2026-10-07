//! Generate deterministic artwork and an isolated playable project, no models.
#[path = "../tests/support/playback.rs"]
mod support;

fn main() -> anyhow::Result<()> {
    let root = storyfella_studio_core::paths::root();
    let directory = root.join(".svs-e2e/inputs/playback");
    let (source, registry) = support::fixture(&directory);
    let path = directory.join("lantern.db");
    let db = storyfella_studio_core::persistence::ProjectDb::open(&path)?;
    db.rename_project("Playback fixture")?;
    db.save_registry(&registry)?;
    std::fs::write(directory.join("lantern.sf"), &source)?;
    let session = storyfella_studio_core::playback::PlaybackSession::compile(&source, &registry)
        .map_err(anyhow::Error::msg)?;
    std::fs::write(
        directory.join("lantern.json"),
        serde_json::to_vec_pretty(session.runtime.plan())?,
    )?;
    println!("{}", path.display());
    Ok(())
}
