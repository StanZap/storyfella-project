use crate::document::{Brush, Color, Layer, Point, Stroke, Tool};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Line,
    Triangle,
    Polygon,
}

impl ShapeKind {
    pub const ALL: [Self; 6] = [
        Self::Rectangle,
        Self::RoundedRectangle,
        Self::Ellipse,
        Self::Line,
        Self::Triangle,
        Self::Polygon,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::RoundedRectangle => "Rounded rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Triangle => "Triangle",
            Self::Polygon => "Pentagon",
        }
    }

    pub fn closed(self) -> bool {
        self != Self::Line
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeStyle {
    pub id: Uuid,
    pub kind: ShapeKind,
    pub fill: Color,
}

impl ShapeStyle {
    pub fn has_pigment(&self, outline: Color) -> bool {
        outline[3] > 0 || (self.kind.closed() && self.fill[3] > 0)
    }
}

pub fn stroke(
    kind: ShapeKind,
    start: Point,
    end: Point,
    fill: Color,
    outline: Color,
    width: f32,
) -> Option<Stroke> {
    let shape = ShapeStyle {
        id: Uuid::new_v4(),
        kind,
        fill,
    };
    let w = (end.x - start.x).abs();
    let h = (end.y - start.y).abs();
    if !shape.has_pigment(outline)
        || !width.is_finite()
        || !(0.1..=512.).contains(&width)
        || ![start.x, start.y, end.x, end.y]
            .iter()
            .all(|n| n.abs() < 1_000_000.)
        || if kind.closed() {
            w < 1. || h < 1.
        } else {
            w.hypot(h) < 1.
        }
    {
        return None;
    }
    let left = start.x.min(end.x);
    let top = start.y.min(end.y);
    let right = left + w;
    let bottom = top + h;
    let point = |x, y| Point { x, y };
    let points = match kind {
        ShapeKind::Line => vec![start, end],
        ShapeKind::Rectangle => vec![
            point(left, top),
            point(right, top),
            point(right, bottom),
            point(left, bottom),
        ],
        ShapeKind::RoundedRectangle => {
            let r = w.min(h) * 0.18;
            [
                (right - r, top + r),
                (right - r, bottom - r),
                (left + r, bottom - r),
                (left + r, top + r),
            ]
            .into_iter()
            .enumerate()
            .flat_map(|(corner, (cx, cy))| {
                (0..=16).map(move |step| {
                    let angle = (corner as f32 - 1. + step as f32 / 16.) * TAU / 4.;
                    point(cx + r * angle.cos(), cy + r * angle.sin())
                })
            })
            .collect()
        }
        ShapeKind::Ellipse | ShapeKind::Triangle | ShapeKind::Polygon => {
            let count = match kind {
                ShapeKind::Triangle => 3,
                ShapeKind::Polygon => 5,
                _ => ((w.max(h) * 5.).sqrt() as usize).clamp(32, 256),
            };
            (0..count)
                .map(|i| {
                    let angle = (i as f32 / count as f32 - 0.25) * TAU;
                    point(
                        left + w * (0.5 + angle.cos() / 2.),
                        top + h * (0.5 + angle.sin() / 2.),
                    )
                })
                .collect()
        }
    };
    Some(Stroke {
        brush: Brush::Ink,
        color: outline,
        shape: Some(shape),
        pressures: Default::default(),
        tool: Tool::Shape,
        size: width,
        opacity: 1.,
        points: points.into(),
    })
}

pub fn margin(stroke: &Stroke) -> f32 {
    if stroke.shape.is_some() && stroke.color[3] == 0 {
        0.
    } else {
        stroke.size / 2.
    }
}

pub fn bounds(stroke: &Stroke) -> Option<(f32, f32, f32, f32)> {
    let first = stroke.points.first()?;
    let (l, t, r, b) = stroke
        .points
        .iter()
        .fold((first.x, first.y, first.x, first.y), |(l, t, r, b), p| {
            (l.min(p.x), t.min(p.y), r.max(p.x), b.max(p.y))
        });
    let m = margin(stroke);
    Some((l - m, t - m, r + m, b + m))
}

pub fn transform(stroke: &mut Stroke, scale: f32, angle: f32, dx: f32, dy: f32) {
    let Some((left, top, right, bottom)) = bounds(stroke) else {
        return;
    };
    let (x, y) = ((left + right) / 2., (top + bottom) / 2.);
    let (sin, cos) = angle.sin_cos();
    for p in std::sync::Arc::make_mut(&mut stroke.points) {
        let (px, py) = ((p.x - x) * scale, (p.y - y) * scale);
        *p = Point {
            x: x + px * cos - py * sin + dx,
            y: y + px * sin + py * cos + dy,
        };
    }
    stroke.size = (stroke.size * scale).clamp(0.1, 512.);
}

pub fn hit(stroke: &Stroke, point: Point, tolerance: f32) -> bool {
    hit_path(stroke, point, tolerance, false)
}

pub fn contains(stroke: &Stroke, point: Point) -> bool {
    hit_path(stroke, point, 0., true)
}

fn hit_path(stroke: &Stroke, point: Point, tolerance: f32, mask: bool) -> bool {
    let Some(shape) = &stroke.shape else {
        return false;
    };
    let mut inside = false;
    let points = stroke.points.as_slice();
    let pairs = points.windows(2).map(|pair| (&pair[0], &pair[1]));
    let closing = points
        .last()
        .zip(points.first())
        .filter(|_| shape.kind.closed());
    for (a, b) in pairs.chain(closing) {
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (b.x - a.x) * (point.y - a.y) / (b.y - a.y)
        {
            inside = !inside;
        }
        if !mask && stroke.color[3] > 0 {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let length = dx * dx + dy * dy;
            if length > 0. {
                let t = (((point.x - a.x) * dx + (point.y - a.y) * dy) / length).clamp(0., 1.);
                if (point.x - a.x - t * dx).hypot(point.y - a.y - t * dy)
                    <= stroke.size / 2. + tolerance
                {
                    return true;
                }
            }
        }
    }
    shape.kind.closed() && (mask || shape.fill[3] > 0) && inside
}

pub fn clip_source(layers: &[Layer], id: Uuid) -> Option<&Stroke> {
    layers
        .iter()
        .filter(|layer| !layer.candidate)
        .flat_map(|layer| &layer.strokes)
        .find(|stroke| {
            stroke
                .shape
                .as_ref()
                .is_some_and(|shape| shape.id == id && shape.kind.closed())
        })
}

pub fn mask(
    stroke: &Stroke,
    width: u32,
    height: u32,
    scale: f32,
    origin: (f32, f32),
) -> anyhow::Result<tiny_skia::Mask> {
    use anyhow::Context as _;
    let mut mask = tiny_skia::Mask::new(width, height).context("Could not allocate shape clip")?;
    if let Some(path) = path(stroke, scale, origin) {
        mask.fill_path(
            &path,
            tiny_skia::FillRule::Winding,
            true,
            tiny_skia::Transform::identity(),
        );
    }
    Ok(mask)
}

fn path(stroke: &Stroke, scale: f32, (x, y): (f32, f32)) -> Option<tiny_skia::Path> {
    let shape = stroke.shape.as_ref()?;
    let first = stroke.points.first()?;
    let mut path = tiny_skia::PathBuilder::new();
    path.move_to(first.x * scale - x, first.y * scale - y);
    for p in &stroke.points[1..] {
        path.line_to(p.x * scale - x, p.y * scale - y);
    }
    if shape.kind.closed() {
        path.close();
    }
    path.finish()
}

pub fn paint(pixmap: &mut tiny_skia::Pixmap, stroke: &Stroke, scale: f32, origin: (f32, f32)) {
    let (Some(shape), Some(path)) = (&stroke.shape, path(stroke, scale, origin)) else {
        return;
    };
    let paint = |[r, g, b, a]: Color| {
        let mut paint = tiny_skia::Paint {
            anti_alias: true,
            ..Default::default()
        };
        paint.set_color_rgba8(r, g, b, (f32::from(a) * stroke.opacity).round() as u8);
        paint
    };
    let transform = tiny_skia::Transform::identity();
    if shape.kind.closed() && shape.fill[3] > 0 {
        pixmap.fill_path(
            &path,
            &paint(shape.fill),
            tiny_skia::FillRule::Winding,
            transform,
            None,
        );
    }
    if stroke.color[3] > 0 {
        pixmap.stroke_path(
            &path,
            &paint(stroke.color),
            &tiny_skia::Stroke {
                width: stroke.size * scale,
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..Default::default()
            },
            transform,
            None,
        );
    }
}
