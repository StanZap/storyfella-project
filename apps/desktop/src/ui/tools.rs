//! The canvas tool rail — direct manipulation of the open image.
//!
//! Every other control in the app is a command (`src/ui/inspector.rs`), and
//! these still resolve to one: the brush paints a mask, and running the edit
//! is `/modify … --mask painted`. What is different is that a tool has
//! *state between clicks* — a stroke is not a command until it is finished
//! — which is why they live in a rail of their own rather than beside the
//! icon actions in the chrome bar.
//!
//! The rail is built to grow. [`Tool`] carries every tool the canvas is
//! planned to have, each with its own icon, shortcut and tooltip, and the
//! ones without an implementation are listed as disabled rather than hidden:
//! a visible, dimmed tool tells you the canvas is going to have it, where an
//! absent one tells you nothing.

use crate::ui::icons::IconName;

/// A canvas tool. `Pointer` is the resting state — no tool active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pointer,
    /// Paint a mask region by hand. The manual counterpart to segmenting
    /// for one (`--mask "her hair"`).
    Brush,
    /// Erase from the painted mask.
    Eraser,
    /// Not yet implemented — see [`Tool::available`].
    Crop,
    Scale,
    Colour,
}

impl Tool {
    /// Every tool in rail order.
    pub const ALL: [Tool; 6] = [
        Tool::Pointer,
        Tool::Brush,
        Tool::Eraser,
        Tool::Crop,
        Tool::Scale,
        Tool::Colour,
    ];

    /// Whether the tool does anything yet. Unavailable tools still appear in
    /// the rail, disabled, so the shape of the toolset is visible.
    pub fn available(&self) -> bool {
        matches!(self, Tool::Pointer | Tool::Brush | Tool::Eraser)
    }

    /// The stable action name, matching the shortcut registry.
    pub fn action(&self) -> &'static str {
        match self {
            Tool::Pointer => "tool-pointer",
            Tool::Brush => "tool-brush",
            Tool::Eraser => "tool-eraser",
            Tool::Crop => "tool-crop",
            Tool::Scale => "tool-scale",
            Tool::Colour => "tool-colour",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tool::Pointer => "Pointer",
            Tool::Brush => "Brush",
            Tool::Eraser => "Eraser",
            Tool::Crop => "Crop",
            Tool::Scale => "Scale",
            Tool::Colour => "Colour",
        }
    }

    pub fn icon(&self) -> IconName {
        match self {
            Tool::Pointer => IconName::Arrow,
            Tool::Brush => IconName::Brush,
            Tool::Eraser => IconName::Eraser,
            Tool::Crop => IconName::Crop,
            Tool::Scale => IconName::Scale,
            Tool::Colour => IconName::Colour,
        }
    }

    /// The single key that selects it, matching the shortcut registry.
    pub fn key(&self) -> char {
        match self {
            // `v` and `e` are already the artifact scope's new-variant and
            // toggle-history; a tool rail is not worth rebinding either.
            Tool::Pointer => 'p',
            Tool::Brush => 'b',
            Tool::Eraser => 'x',
            Tool::Crop => 'c',
            // `t` for transform: `s` is the tail of the `g s` chord.
            Tool::Scale => 't',
            Tool::Colour => 'k',
        }
    }

    /// The tool a plan names, by its lowercase label.
    pub fn parse(name: &str) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.label().eq_ignore_ascii_case(name))
    }

    /// The tool a key selects, if it selects one and that tool works.
    pub fn from_key(key: char) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.key() == key && tool.available())
    }

    /// Whether the tool paints onto the mask overlay.
    pub fn paints(&self) -> bool {
        matches!(self, Tool::Brush | Tool::Eraser)
    }
}

/// Where the image actually sits inside its pane, and how big it is.
///
/// The render pane letterboxes: `object-contain` scales the image to fit and
/// centres the remainder, so a pointer position in pane coordinates is not a
/// position in image coordinates. Every stroke has to cross this, and an
/// error here is invisible — the overlay draws where you pointed while the
/// mask lands somewhere else — which is why it is a pure function with
/// tests rather than arithmetic inline in an event handler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fitted {
    /// Pane-space offset of the image's top-left corner.
    pub left: f64,
    pub top: f64,
    /// Pane-space size of the drawn image.
    pub width: f64,
    pub height: f64,
}

impl Fitted {
    /// Fits `image` inside `pane`, both in pixels, preserving aspect ratio.
    pub fn contain(pane: (f64, f64), image: (u32, u32)) -> Option<Fitted> {
        let (pane_width, pane_height) = pane;
        let (image_width, image_height) = (image.0 as f64, image.1 as f64);
        if pane_width <= 0.0 || pane_height <= 0.0 || image_width <= 0.0 || image_height <= 0.0 {
            return None;
        }
        let scale = (pane_width / image_width).min(pane_height / image_height);
        let width = image_width * scale;
        let height = image_height * scale;
        Some(Fitted {
            left: (pane_width - width) / 2.0,
            top: (pane_height - height) / 2.0,
            width,
            height,
        })
    }

    /// Maps a pane-space point to image pixels, or `None` when it falls in
    /// the letterbox rather than on the picture.
    pub fn to_image(&self, x: f64, y: f64, image: (u32, u32)) -> Option<(f64, f64)> {
        let local_x = x - self.left;
        let local_y = y - self.top;
        if local_x < 0.0 || local_y < 0.0 || local_x > self.width || local_y > self.height {
            return None;
        }
        Some((
            local_x / self.width * image.0 as f64,
            local_y / self.height * image.1 as f64,
        ))
    }

    /// A brush radius given in pane pixels, in image pixels — so the brush
    /// stays the size it looks whatever the zoom.
    pub fn radius_to_image(&self, radius: f64, image: (u32, u32)) -> f64 {
        if self.width <= 0.0 {
            return radius;
        }
        radius / self.width * image.0 as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::shortcuts;

    #[test]
    fn a_wide_pane_letterboxes_left_and_right() {
        // 1000x600 pane, 2:3 portrait: height-limited, so bars on the sides.
        let fitted = Fitted::contain((1000.0, 600.0), (1024, 1536)).unwrap();
        assert_eq!(fitted.height, 600.0);
        assert_eq!(fitted.width, 400.0);
        assert_eq!(fitted.left, 300.0);
        assert_eq!(fitted.top, 0.0);

        // The image's own corners map to its own corners.
        let image = (1024, 1536);
        let (x, y) = fitted.to_image(300.0, 0.0, image).unwrap();
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9);
        let (x, y) = fitted.to_image(700.0, 600.0, image).unwrap();
        assert!((x - 1024.0).abs() < 1e-9 && (y - 1536.0).abs() < 1e-9);

        // The centre of the pane is the centre of the picture.
        let (x, y) = fitted.to_image(500.0, 300.0, image).unwrap();
        assert!((x - 512.0).abs() < 1e-9 && (y - 768.0).abs() < 1e-9);

        // A point in the letterbox is not a point on the picture. Without
        // this a stroke started beside the image would clamp onto its edge.
        assert_eq!(fitted.to_image(100.0, 300.0, image), None);
    }

    #[test]
    fn a_tall_pane_letterboxes_top_and_bottom() {
        let fitted = Fitted::contain((400.0, 1000.0), (1024, 1536)).unwrap();
        assert_eq!(fitted.width, 400.0);
        assert_eq!(fitted.height, 600.0);
        assert_eq!(fitted.left, 0.0);
        assert_eq!(fitted.top, 200.0);
        assert_eq!(fitted.to_image(200.0, 100.0, (1024, 1536)), None);
    }

    #[test]
    fn a_brush_keeps_its_apparent_size() {
        // The image is drawn at 400px wide from 1024 real pixels, so a 10px
        // brush on screen has to be 25.6 image pixels to look the same.
        let fitted = Fitted::contain((1000.0, 600.0), (1024, 1536)).unwrap();
        let radius = fitted.radius_to_image(10.0, (1024, 1536));
        assert!((radius - 25.6).abs() < 1e-9, "got {radius}");
    }

    #[test]
    fn a_collapsed_pane_has_no_fit() {
        assert_eq!(Fitted::contain((0.0, 600.0), (1024, 1536)), None);
        assert_eq!(Fitted::contain((1000.0, 600.0), (0, 1536)), None);
    }

    #[test]
    fn every_tool_has_a_shortcut_row_that_matches_its_key() {
        for tool in Tool::ALL {
            let keys = shortcuts::keys(tool.action()).unwrap_or_else(|| {
                panic!("{} has no shortcut row", tool.action());
            });
            assert_eq!(
                keys,
                tool.key().to_string(),
                "{} advertises a different key than the rail binds",
                tool.action()
            );
        }
    }

    #[test]
    fn tool_keys_are_unique() {
        let mut keys: Vec<char> = Tool::ALL.iter().map(|tool| tool.key()).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "two tools share a key");
    }

    #[test]
    fn unimplemented_tools_are_not_selectable_by_key() {
        assert_eq!(Tool::from_key('b'), Some(Tool::Brush));
        assert_eq!(
            Tool::from_key('c'),
            None,
            "crop is listed but not implemented, so its key must not select it"
        );
    }
}
