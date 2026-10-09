//! Append-stable curves: new input never changes geometry already painted.
use crate::document::Stroke;

pub fn segments(stroke: &Stroke, from: usize) -> Vec<[f32; 6]> {
    let mut output = Vec::new();
    let pressure = |i| stroke.pressures.get(i).copied().unwrap_or(1.);
    if from == 0 {
        if let Some(p) = stroke.points.first() {
            output.push([p.x, p.y, p.x, p.y, pressure(0), pressure(0)]);
        }
    }
    for i in from.max(1)..stroke.points.len() {
        let a = stroke.points[i - 1];
        let b = stroke.points[i];
        let delta = [b.x - a.x, b.y - a.y];
        let length = delta[0].hypot(delta[1]);
        let mut tangent = delta;
        if i > 1 && length > 0.001 {
            let previous = stroke.points[i - 2];
            let incoming = [a.x - previous.x, a.y - previous.y];
            let previous_length = incoming[0].hypot(incoming[1]);
            // Preserve reversals and turns of 60 degrees or more. Limit handles
            // after uneven samples so short movements cannot produce large loops.
            if previous_length > 0.001
                && (incoming[0] * delta[0] + incoming[1] * delta[1]) / (previous_length * length)
                    > 0.5
            {
                let handle = previous_length.min(length);
                tangent = [
                    incoming[0] * handle / previous_length,
                    incoming[1] * handle / previous_length,
                ];
            }
        }
        let c1 = [a.x + tangent[0] / 3., a.y + tangent[1] / 3.];
        let c2 = [b.x - delta[0] / 3., b.y - delta[1] / 3.];
        let bend = (tangent[0] - delta[0]).hypot(tangent[1] - delta[1]);
        // Bound curve approximation error without subdividing long straight lines.
        let steps = (bend / 0.5).sqrt().ceil().clamp(1., 64.) as usize;
        let mut last = [a.x, a.y, pressure(i - 1)];
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let u = 1. - t;
            let next = [
                u * u * u * a.x + 3. * u * u * t * c1[0] + 3. * u * t * t * c2[0] + t * t * t * b.x,
                u * u * u * a.y + 3. * u * u * t * c1[1] + 3. * u * t * t * c2[1] + t * t * t * b.y,
                pressure(i - 1) + (pressure(i) - pressure(i - 1)) * t,
            ];
            output.push([last[0], last[1], next[0], next[1], last[2], next[2]]);
            last = next;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Brush, Point, Tool};
    fn stroke(points: &[[f32; 2]]) -> Stroke {
        Stroke {
            brush: Brush::Graphite,
            tool: Tool::Pencil,
            size: 5.,
            opacity: 0.8,
            points: points
                .iter()
                .map(|p| Point { x: p[0], y: p[1] })
                .collect::<Vec<_>>()
                .into(),
            pressures: (0..points.len())
                .map(|i| 0.2 + i as f32 * 0.2)
                .collect::<Vec<_>>()
                .into(),
        }
    }
    #[test]
    fn sparse_bends_are_curved_with_continuous_pressure() {
        let s = stroke(&[[0., 0.], [30., 0.], [60., 20.]]);
        let curve = segments(&s, 2);
        assert!(curve.len() > 1);
        let first = curve[0];
        assert!((first[3] - first[1]) / (first[2] - first[0]) < 0.3);
        assert_eq!(&first[..2], &[30., 0.]);
        assert_eq!(&curve.last().unwrap()[2..4], &[60., 20.]);
        for pair in curve.windows(2) {
            assert_eq!(pair[0][2], pair[1][0]);
            assert_eq!(pair[0][3], pair[1][1]);
            assert_eq!(pair[0][5], pair[1][4]);
        }
        assert!(curve.iter().all(|p| p[4] <= p[5] && p[5] <= 0.6));
    }
    #[test]
    fn appending_points_preserves_published_geometry() {
        let mut s = stroke(&[[0., 0.], [30., 0.], [60., 20.]]);
        let original = segments(&s, 0);
        std::sync::Arc::make_mut(&mut s.points).push(Point { x: 90., y: 50. });
        std::sync::Arc::make_mut(&mut s.pressures).push(0.8);
        let mut incremental = original.clone();
        incremental.extend(segments(&s, 3));
        assert_eq!(segments(&s, 0), incremental);
        assert_eq!(&segments(&s, 0)[..original.len()], original);
    }
    #[test]
    fn corners_reversals_and_repeated_points_stay_bounded() {
        for points in [
            vec![[0., 0.], [30., 0.], [30., 30.]],
            vec![[0., 0.], [30., 0.], [0., 0.]],
            vec![[0., 0.], [0., 0.], [30., 0.]],
        ] {
            let s = stroke(&points);
            let curve = segments(&s, 2);
            assert_eq!(curve.len(), 1);
            assert!(curve[0].iter().all(|v| v.is_finite()));
            assert_eq!(&curve[0][2..4], &points[2]);
        }
    }
}
