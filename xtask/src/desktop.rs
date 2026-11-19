//! Linux desktop integration: the entry and icon desktop search finds an app by.
//!
//! Both live under `$XDG_DATA_HOME`, else `~/.local/share`: the entry as
//! `applications/<id>.desktop`, the icon as `<id>.png` in the hicolor theme.
//! The id is the app's bundle identifier, which the app also sets as its window
//! `app_id`, so a running window is matched to the entry and shows its icon.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// What a desktop entry says about one installed app.
pub struct Launcher<'a> {
    /// Reverse-DNS id: the entry and icon file names and the window `app_id`.
    pub id: &'a str,
    /// The name shown in menus and search.
    pub name: &'a str,
    /// The one-line description shown beside the name.
    pub comment: &'a str,
    /// `Categories=` value, `;`-terminated.
    pub categories: &'a str,
    /// The installed executable the entry launches.
    pub exec: &'a Path,
    /// A square PNG, installed as the app's icon.
    pub icon: &'a Path,
}

/// The per-user data directory entries and icons are installed into.
pub fn data_home() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty()).context(
        "neither XDG_DATA_HOME nor HOME is set, so there is no per-user data directory for the desktop entry",
    )?;
    Ok(PathBuf::from(home).join(".local/share"))
}

/// Install the icon and desktop entry under `data_home`, replacing earlier
/// copies, and return the entry's path.
pub fn install(launcher: &Launcher, data_home: &Path) -> Result<PathBuf> {
    let png = fs::read(launcher.icon)
        .with_context(|| format!("reading the icon {}", launcher.icon.display()))?;
    let width = square_png_width(&png)
        .with_context(|| format!("installing the icon {}", launcher.icon.display()))?;
    let icon_dir = data_home.join(format!("icons/hicolor/{width}x{width}/apps"));
    fs::create_dir_all(&icon_dir).with_context(|| format!("creating {}", icon_dir.display()))?;
    let icon = icon_dir.join(format!("{}.png", launcher.id));
    fs::write(&icon, png).with_context(|| format!("installing {}", icon.display()))?;

    let applications = data_home.join("applications");
    fs::create_dir_all(&applications)
        .with_context(|| format!("creating {}", applications.display()))?;
    let entry = applications.join(format!("{}.desktop", launcher.id));
    fs::write(&entry, desktop_entry(launcher))
        .with_context(|| format!("writing {}", entry.display()))?;
    Ok(entry)
}

fn desktop_entry(launcher: &Launcher) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Comment={comment}\n\
         Exec={exec}\n\
         Icon={id}\n\
         Terminal=false\n\
         Categories={categories}\n\
         StartupWMClass={id}\n",
        name = launcher.name,
        comment = launcher.comment,
        exec = exec_arg(launcher.exec),
        id = launcher.id,
        categories = launcher.categories,
    )
}

/// `path` quoted for an `Exec=` key, which reserves `"`, `` ` ``, `$`, `\`
/// inside quotes and `%` everywhere.
fn exec_arg(path: &Path) -> String {
    let mut quoted = String::from("\"");
    for character in path.to_string_lossy().chars() {
        match character {
            '"' | '`' | '$' | '\\' => {
                quoted.push_str("\\\\");
                quoted.push(character);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// The width of a square PNG, read from its header; the hicolor theme files
/// icons by size.
fn square_png_width(png: &[u8]) -> Result<u32> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 24 || !png.starts_with(SIGNATURE) || &png[12..16] != b"IHDR" {
        bail!("not a PNG");
    }
    let width = u32::from_be_bytes(png[16..20].try_into().expect("four bytes"));
    let height = u32::from_be_bytes(png[20..24].try_into().expect("four bytes"));
    if width != height {
        bail!("an icon must be square, not {width}x{height}");
    }
    Ok(width)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 24-byte header of a `width`x`height` PNG: all `square_png_width` reads.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png
    }

    #[test]
    fn installs_the_entry_and_files_the_icon_by_size() {
        let root = tempfile::tempdir().unwrap();
        let icon = root.path().join("app.png");
        fs::write(&icon, png_header(256, 256)).unwrap();
        let data = root.path().join("share");
        let launcher = Launcher {
            id: "com.example.app",
            name: "Example",
            comment: "Does things",
            categories: "Graphics;",
            exec: Path::new("/home/me/.local/bin/example"),
            icon: &icon,
        };

        let entry = install(&launcher, &data).unwrap();

        assert_eq!(entry, data.join("applications/com.example.app.desktop"));
        let text = fs::read_to_string(&entry).unwrap();
        for line in [
            "[Desktop Entry]",
            "Name=Example",
            "Exec=\"/home/me/.local/bin/example\"",
            "Icon=com.example.app",
            "StartupWMClass=com.example.app",
        ] {
            assert!(
                text.lines().any(|candidate| candidate == line),
                "{line} in\n{text}"
            );
        }
        assert_eq!(
            fs::read(data.join("icons/hicolor/256x256/apps/com.example.app.png")).unwrap(),
            png_header(256, 256)
        );
    }

    #[test]
    fn exec_paths_escape_reserved_characters() {
        assert_eq!(
            exec_arg(Path::new("/opt/100% \"a\"/$x")),
            r#""/opt/100%% \\"a\\"/\\$x""#
        );
    }

    #[test]
    fn an_icon_that_is_not_a_square_png_is_refused_before_anything_is_written() {
        let root = tempfile::tempdir().unwrap();
        let icon = root.path().join("app.png");
        let data = root.path().join("share");
        let launcher = |icon| Launcher {
            id: "com.example.app",
            name: "Example",
            comment: "",
            categories: "Graphics;",
            exec: Path::new("/bin/example"),
            icon,
        };

        fs::write(&icon, png_header(256, 128)).unwrap();
        let error = install(&launcher(&icon), &data).unwrap_err();
        assert!(format!("{error:#}").contains("square"), "{error:#}");

        fs::write(&icon, b"GIF89a").unwrap();
        let error = install(&launcher(&icon), &data).unwrap_err();
        assert!(format!("{error:#}").contains("not a PNG"), "{error:#}");

        assert!(!data.exists());
    }
}
