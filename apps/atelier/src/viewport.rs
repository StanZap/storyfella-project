use crate::document::Point;
#[derive(Clone, Copy)]
pub struct Viewport {
    pub zoom: f32,
    pub pan: Point,
    pub turns: u8,
    pub mirror: bool,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: 1.,
            pan: Point { x: 0., y: 0. },
            turns: 0,
            mirror: false,
        }
    }
}
impl Viewport {
    pub fn dimensions(&self, w: u32, h: u32) -> (f32, f32) {
        if self.turns % 2 == 1 {
            (h as f32, w as f32)
        } else {
            (w as f32, h as f32)
        }
    }
    pub fn scale(&self, available: (f32, f32), dimensions: (u32, u32)) -> f32 {
        let (w, h) = self.dimensions(dimensions.0, dimensions.1);
        ((available.0 - 64.).max(1.) / w).min((available.1 - 64.).max(1.) / h) * self.zoom
    }
    pub fn to_document(self, p: Point, w: f32, h: f32) -> Point {
        let (vw, _) = self.dimensions(w as u32, h as u32);
        let x = if self.mirror { vw - p.x } else { p.x };
        match self.turns % 4 {
            0 => Point { x, y: p.y },
            1 => Point { x: p.y, y: h - x },
            2 => Point {
                x: w - x,
                y: h - p.y,
            },
            _ => Point { x: w - p.y, y: x },
        }
    }
    pub fn to_view(self, p: Point, w: f32, h: f32) -> Point {
        let mut p = match self.turns % 4 {
            0 => p,
            1 => Point { x: h - p.y, y: p.x },
            2 => Point {
                x: w - p.x,
                y: h - p.y,
            },
            _ => Point { x: p.y, y: w - p.x },
        };
        if self.mirror {
            p.x = self.dimensions(w as u32, h as u32).0 - p.x;
        }
        p
    }
}
