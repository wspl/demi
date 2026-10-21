//! A raw picture as NV12, the input both Media Foundation's and VA-API's
//! encoders take: a plane of luma, then one of interleaved chroma at half the
//! size each way. BT.601, limited range, which is what the pictures are read
//! back with (`crates/shared-recording/src/export.rs`).

use yuv::{YuvBiPlanarImageMut, YuvConversionMode, YuvRange, YuvStandardMatrix, bgra_to_yuv_nv12, rgb_to_yuv_nv12, rgba_to_yuv_nv12};

use crate::{Error, PixelFormat, Raw, Result};

/// An NV12 picture's planes, kept for the next.
pub struct Nv12 {
    image: YuvBiPlanarImageMut<'static, u8>,
}

impl Nv12 {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            image: YuvBiPlanarImageMut::alloc(width, height, yuv::YuvChromaSubsampling::Yuv420),
        }
    }

    /// Converts `frame`, which has this picture's size, into it.
    pub fn read(&mut self, frame: Raw<'_>) -> Result<()> {
        let stride = frame.width * frame.format.bytes() as u32;
        let convert = match frame.format {
            PixelFormat::Rgb => rgb_to_yuv_nv12,
            PixelFormat::Rgba => rgba_to_yuv_nv12,
            PixelFormat::Bgra => bgra_to_yuv_nv12,
        };
        convert(
            &mut self.image,
            frame.pixels,
            stride,
            YuvRange::Limited,
            YuvStandardMatrix::Bt601,
            YuvConversionMode::Balanced,
        )
        .map_err(|error| Error(format!("a picture did not convert to NV12: {error}")))
    }

    /// The luma plane and its stride, then the chroma plane and its stride.
    pub fn planes(&self) -> ((&[u8], usize), (&[u8], usize)) {
        (
            (self.image.y_plane.borrow(), self.image.y_stride as usize),
            (self.image.uv_plane.borrow(), self.image.uv_stride as usize),
        )
    }
}
