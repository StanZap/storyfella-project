//! Portable, approved artwork exchanged between Atelier and Studio. Importing
//! creates a completed registry revision; it never calls an inference provider.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovedArtwork {
    pub version: u32,
    pub name: String,
    pub image: String,
    pub prompt: String,
    pub provenance: String,
}
impl ApprovedArtwork {
    pub fn image(&self) -> Result<image::DynamicImage> {
        ensure!(self.version == 1, "Unsupported artwork package version");
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 512,
            "Artwork needs a name of at most 512 bytes"
        );
        ensure!(
            self.prompt.len() <= 32_768 && self.provenance.len() <= 32_768,
            "Artwork metadata exceeds 32 KB"
        );
        ensure!(
            self.image.len() <= 28 * 1024 * 1024,
            "Artwork exceeds 20 MB"
        );
        let bytes = STANDARD
            .decode(&self.image)
            .context("Invalid artwork encoding")?;
        ensure!(bytes.len() <= 20 * 1024 * 1024, "Artwork exceeds 20 MB");
        let mut reader =
            image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().context("Unsupported or damaged artwork")?;
        ensure!(
            image.width() as u64 * image.height() as u64 <= 32_000_000,
            "Artwork exceeds 32 megapixels"
        );
        Ok(image)
    }
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            fs::metadata(path)?.len() <= 28 * 1024 * 1024,
            "Artwork package exceeds 28 MB"
        );
        let artwork: Self = serde_json::from_slice(&fs::read(path)?)?;
        artwork.image()?;
        Ok(artwork)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.image()?;
        write_atomic(path, &serde_json::to_vec(self)?)
    }
    pub fn write_asset(&self, directory: &Path) -> Result<PathBuf> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        self.image()?
            .write_to(&mut bytes, image::ImageFormat::Png)?;
        fs::create_dir_all(directory)?;
        let path = directory.join(format!("{}.png", Uuid::new_v4()));
        write_atomic(&path, &bytes.into_inner())?;
        Ok(fs::canonicalize(path)?)
    }
    pub fn preview_data_url(&self) -> Result<String> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        self.image()?
            .thumbnail(384, 384)
            .write_to(&mut bytes, image::ImageFormat::Png)?;
        Ok(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes.into_inner())
        ))
    }
}
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.partial", Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packages_preserve_pixels_provenance_and_portable_full_resolution() {
        let directory = std::env::temp_dir().join(format!("approved-artwork-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let image = image::RgbaImage::from_pixel(2400, 1200, image::Rgba([10, 40, 90, 128]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let artwork = ApprovedArtwork {
            version: 1,
            name: "Cabin".into(),
            image: STANDARD.encode(bytes.into_inner()),
            prompt: "Change the sky".into(),
            provenance: "Replicate · Qwen · parent original".into(),
        };
        let package = directory.join("approved.sfart");
        artwork.save(&package).unwrap();
        let loaded = ApprovedArtwork::load(&package).unwrap();
        assert_eq!(loaded, artwork);
        let asset = loaded.write_asset(&directory.join("generated")).unwrap();
        assert!(asset.is_absolute());
        assert_eq!(image::open(asset).unwrap().to_rgba8(), image);
        fs::write(&package, b"not an artwork package").unwrap();
        assert!(ApprovedArtwork::load(&package).is_err());
        let mut invalid = artwork;
        invalid.version = 2;
        assert!(invalid.save(&package).is_err());
        assert_eq!(fs::read(package).unwrap(), b"not an artwork package");
        fs::remove_dir_all(directory).unwrap();
    }
}
