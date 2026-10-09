# Atelier shared-surface patch

Vendored from crates.io gpui-pre-apple 0.3.7 (Apache-2.0), Zed snapshot
1a28cff4b409169bac058bca40dfbfeb7621d19b.

Local changes are limited to `src/metal_renderer.rs` and `src/shaders.metal`:
add a premultiplied BGRA Core Video surface pipeline, preserving the existing
NV12 video pipeline and surface clipping/z-order. The root Cargo patch pins all
workspace consumers to this source. BGRA surfaces must be Metal-compatible and
IOSurface-backed, and remain immutable after submission.

The renderer retains pixel buffers and Core Video texture mappings until the
Metal command buffer completes, then releases them. Cache flushing prevents
unused mappings from accumulating across frames.
