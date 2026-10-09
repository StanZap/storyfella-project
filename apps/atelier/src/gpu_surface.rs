//! Immutable IOSurface-backed tiles shared with GPUI; no CPU pixel access in presentation.
use anyhow::Result;
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_video::{
    metal_texture::{CVMetalTexture, CVMetalTextureGetTexture, CVMetalTextureKeys},
    metal_texture_cache::CVMetalTextureCache,
    pixel_buffer::{kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferKeys},
};
use metal::{foreign_types::ForeignTypeRef, MTLPixelFormat, MTLTextureUsage, Texture};
pub struct Surface {
    buffer: CVPixelBuffer,
    pub texture: Texture,
    _mapping: CVMetalTexture,
}
// Core Video's reference counting is thread-safe. Surfaces are published only
// after GPU completion, and never written again. Retained handles own the storage.
unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}
impl Surface {
    pub fn new(cache: &CVMetalTextureCache, w: u32, h: u32) -> Result<Self> {
        let empty = CFDictionary::<CFString, CFType>::from_CFType_pairs(&[]);
        let options = CFDictionary::from_CFType_pairs(&[
            (
                CFString::from(CVPixelBufferKeys::MetalCompatibility),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
                empty.as_CFType(),
            ),
        ]);
        let buffer = CVPixelBuffer::new(
            kCVPixelFormatType_32BGRA,
            w as usize,
            h as usize,
            Some(&options),
        )
        .map_err(|code| anyhow::anyhow!("Core Video surface allocation: {code}"))?;
        let attributes = CFDictionary::from_CFType_pairs(&[(
            CFString::from(CVMetalTextureKeys::Usage),
            CFNumber::from(
                (MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite).bits() as i64,
            )
            .as_CFType(),
        )]);
        let mapping = cache
            .create_texture_from_image(
                buffer.as_concrete_TypeRef(),
                Some(&attributes),
                MTLPixelFormat::BGRA8Unorm,
                w as usize,
                h as usize,
                0,
            )
            .map_err(|code| anyhow::anyhow!("Core Video Metal mapping: {code}"))?;
        let ptr = unsafe { CVMetalTextureGetTexture(mapping.as_concrete_TypeRef()) };
        anyhow::ensure!(!ptr.is_null(), "Core Video returned no Metal texture");
        // Borrow the get-rule pointer, then explicitly retain it. Do not take create-rule ownership.
        let texture = unsafe { metal::TextureRef::from_ptr(ptr.cast()) }.to_owned();
        Ok(Self {
            buffer,
            texture,
            _mapping: mapping,
        })
    }
    pub fn buffer(&self) -> CVPixelBuffer {
        self.buffer.clone()
    }
    pub fn width(&self) -> u32 {
        self.texture.width() as u32
    }
    pub fn height(&self) -> u32 {
        self.texture.height() as u32
    }
}
