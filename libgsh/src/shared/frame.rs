use crate::shared::protocol::frame::Segment;
use std::mem;

/// Ping-pong storage for previous frames without full-frame copies.
/// After diffing and sending, use `cur_frame = prev_frame.update_with_frame(cur_frame)`
/// to retain the sent frame and reuse the old buffer.
#[derive(Debug, Clone)]
pub struct PrevFrame {
    buf0: Vec<u8>,
    buf1: Vec<u8>,
    cur: usize,
}

impl PrevFrame {
    pub fn new() -> Self {
        Self {
            buf0: Vec::new(),
            buf1: Vec::new(),
            cur: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf0: Vec::with_capacity(cap),
            buf1: Vec::with_capacity(cap),
            cur: 0,
        }
    }

    /// Returns a slice to the current previous frame.
    pub fn current(&self) -> &[u8] {
        if self.cur == 0 {
            &self.buf0
        } else {
            &self.buf1
        }
    }

    /// Stores `new_frame` and returns the old buffer for reuse without copying.
    pub fn update_with_frame(&mut self, new_frame: Vec<u8>) -> Vec<u8> {
        if self.cur == 0 {
            let old = mem::replace(&mut self.buf1, new_frame);
            self.cur = 1;
            old
        } else {
            let old = mem::replace(&mut self.buf0, new_frame);
            self.cur = 0;
            old
        }
    }
}

pub fn full_frame_segment(
    full_frame_data: &[u8],
    frame_width: usize,
    frame_height: usize,
) -> Vec<Segment> {
    vec![Segment {
        x: 0,
        y: 0,
        width: frame_width as u32,
        height: frame_height as u32,
        data: full_frame_data.to_vec(),
    }]
}

/// Returns changed rectangular regions compared with `prev_frame`.
/// The caller must update `prev_frame` after sending the frame.
pub fn optimize_segments(
    full_frame_data: &[u8],
    frame_width: usize,
    frame_height: usize,
    prev_frame: &PrevFrame,
    pixel_bytes: usize,
) -> Vec<Segment> {
    const MIN_SEGMENT_ROWS: usize = 4;
    const MAX_SEGMENT_COUNT: usize = 50;
    let mut optimized_segments = Vec::new();
    let mut current_segment: Option<Segment> = None;

    for y in 0..frame_height {
        let start = y * frame_width * pixel_bytes;
        let end = start + frame_width * pixel_bytes;
        if let Some(prev_row) = prev_frame.current().get(start..end) {
            if *prev_row != full_frame_data[start..end] {
                let segment_data = full_frame_data[start..end].to_vec();
                if let Some(ref mut segment) = current_segment {
                    if segment.y + segment.height as i32 == y as i32
                        && segment.width as usize == frame_width
                    {
                        segment.height += 1;
                        segment.data.extend(segment_data);
                    } else {
                        if optimized_segments.len() + 1 > MAX_SEGMENT_COUNT {
                            return full_frame_segment(full_frame_data, frame_width, frame_height);
                        }
                        if segment.height as usize >= MIN_SEGMENT_ROWS {
                            optimized_segments.push(segment.clone());
                        }
                        *segment = Segment {
                            x: 0,
                            y: y as i32,
                            width: frame_width as u32,
                            height: 1,
                            data: segment_data,
                        };
                    }
                } else {
                    current_segment = Some(Segment {
                        x: 0,
                        y: y as i32,
                        width: frame_width as u32,
                        height: 1,
                        data: segment_data,
                    });
                }
            }
        } else {
            let segment_data = full_frame_data[start..end].to_vec();
            if let Some(ref mut segment) = current_segment {
                if segment.y + segment.height as i32 == y as i32
                    && segment.width as usize == frame_width
                {
                    segment.height += 1;
                    segment.data.extend(segment_data);
                } else {
                    if optimized_segments.len() + 1 > MAX_SEGMENT_COUNT {
                        return full_frame_segment(full_frame_data, frame_width, frame_height);
                    }
                    if segment.height as usize >= MIN_SEGMENT_ROWS {
                        optimized_segments.push(segment.clone());
                    }
                    *segment = Segment {
                        x: 0,
                        y: y as i32,
                        width: frame_width as u32,
                        height: 1,
                        data: segment_data,
                    };
                }
            } else {
                current_segment = Some(Segment {
                    x: 0,
                    y: y as i32,
                    width: frame_width as u32,
                    height: 1,
                    data: segment_data,
                });
            }
        }
    }

    if let Some(segment) = current_segment {
        optimized_segments.push(segment);
    }

    optimized_segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_frame_segment() {
        let data = vec![255u8; 100 * 100 * 4]; // 100x100 RGBA
        let segments = full_frame_segment(&data, 100, 100);

        assert_eq!(segments.len(), 1);
        let segment = &segments[0];
        assert_eq!(segment.x, 0);
        assert_eq!(segment.y, 0);
        assert_eq!(segment.width, 100);
        assert_eq!(segment.height, 100);
        assert_eq!(segment.data.len(), 100 * 100 * 4);
    }

    #[test]
    fn test_optimize_segments_identical_frames() {
        let width = 10;
        let height = 10;
        let pixel_bytes = 4;
        let data = vec![128u8; width * height * pixel_bytes];
        let mut prev_frame = PrevFrame::new();
        prev_frame.update_with_frame(data.clone());

        let segments = optimize_segments(&data, width, height, &prev_frame, pixel_bytes);

        assert_eq!(segments.len(), 0);
    }

    #[test]
    fn test_optimize_segments_single_row_change() {
        let width = 10;
        let height = 10;
        let pixel_bytes = 4;
        let mut data = vec![128u8; width * height * pixel_bytes];
        let mut prev_frame = PrevFrame::new();
        prev_frame.update_with_frame(vec![128u8; width * height * pixel_bytes]);

        for i in 0..width * pixel_bytes {
            data[5 * width * pixel_bytes + i] = 255;
        }

        let segments = optimize_segments(&data, width, height, &prev_frame, pixel_bytes);

        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_optimize_segments_large_change() {
        let width = 10;
        let height = 10;
        let pixel_bytes = 4;
        let data = vec![128u8; width * height * pixel_bytes];
        let mut prev_frame = PrevFrame::new();
        prev_frame.update_with_frame(vec![255u8; width * height * pixel_bytes]);

        let segments = optimize_segments(&data, width, height, &prev_frame, pixel_bytes);

        assert!(segments.len() > 0);
    }

    #[test]
    fn test_frame_segment_data_integrity() {
        let width = 5;
        let height = 5;
        let pixel_bytes = 4;
        let mut data = vec![0u8; width * height * pixel_bytes];

        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * pixel_bytes;
                data[idx] = (x * 255 / width) as u8; // R
                data[idx + 1] = (y * 255 / height) as u8; // G
                data[idx + 2] = 128; // B
                data[idx + 3] = 255; // A
            }
        }

        let segments = full_frame_segment(&data, width, height);
        assert_eq!(segments[0].data, data);
    }
}
