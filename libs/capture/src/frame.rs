//! Frame types shared between capture backends and the encoder.

/// Pixel layout of [`VideoFrame::data`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    /// 8 bits per channel, BGRA byte order — what every OS compositor
    /// hands us by default; conversion happens at the encode stage.
    Bgra8,
}

/// One captured frame, in physical pixels.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Capture timestamp in microseconds on the pipeline's monotonic clock.
    pub pts_us: u64,
    pub width: u32,
    pub height: u32,
    /// Bytes per row; may include padding.
    pub stride: u32,
    pub format: FrameFormat,
    /// Pixel buffer, `stride * height` bytes long.
    pub data: Vec<u8>,
}

impl VideoFrame {
    /// Length `data` must have for the current geometry.
    pub fn expected_len(&self) -> usize {
        self.stride as usize * self.height as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_len_accounts_for_stride() {
        let frame = VideoFrame {
            pts_us: 0,
            width: 100,
            height: 50,
            stride: 128 * 4,
            format: FrameFormat::Bgra8,
            data: vec![0; 128 * 4 * 50],
        };
        assert_eq!(frame.expected_len(), frame.data.len());
    }
}
