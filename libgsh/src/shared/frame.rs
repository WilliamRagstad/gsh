use crate::shared::protocol::frame::Segment;
use std::mem;

/// A ping-pong previous-frame buffer to avoid full-frame copies between frames.
///
/// Usage pattern:
/// - Service/example keeps a `cur_frame: Vec<u8>` that it fills each tick.
/// - Call `optimize_segments(&cur_frame, ..., &mut prev_frame, pixel_bytes)` to compute diffs
///   against the current previous-frame buffer.
/// - After sending the frame, call `cur_frame = prev_frame.update_with_frame(cur_frame)`
///   which moves ownership of `cur_frame` into the prev-frame storage and returns the
///   old previous-frame Vec for reuse as the next `cur_frame`. This avoids copying the
///   entire frame buffer every tick.
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

    /// Update the ping-pong buffers by taking ownership of `new_frame`.
    /// Returns the old buffer (the one that becomes the new current frame to be filled).
    /// This avoids allocating/copying a fresh Vec every tick.
    pub fn update_with_frame(&mut self, new_frame: Vec<u8>) -> Vec<u8> {
        if self.cur == 0 {
            // buf1 becomes current; put new_frame into buf0 and return old buf1
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

/// A function to optimize a frame segments for transmission.
/// Identifying what partial (rectangle-area) updates are needed to be sent to the client compared to the previous frame.
pub fn optimize_segments(
    full_frame_data: &[u8],
    frame_width: usize,
    frame_height: usize,
    prev_frame: &PrevFrame,
    pixel_bytes: usize,
) -> Vec<Segment> {
    const MIN_SEGMENT_ROWS: usize = 4; // Minimum segment size in rows
    const MAX_SEGMENT_COUNT: usize = 50; // Maximum number of segments to send
    let mut optimized_segments = Vec::new();
    let mut current_segment: Option<Segment> = None;

    // Compare the new segment with the previous one and find differences
    for y in 0..frame_height {
        let start = y * frame_width * pixel_bytes;
        let end = start + frame_width * pixel_bytes;
        if let Some(prev_row) = prev_frame.current().get(start..end) {
            if *prev_row != full_frame_data[start..end] {
                let segment_data = full_frame_data[start..end].to_vec();
                if let Some(ref mut segment) = current_segment {
                    // Extend the current segment if it's contiguous
                    if segment.y + segment.height as i32 == y as i32
                        && segment.width as usize == frame_width
                    {
                        segment.height += 1;
                        segment.data.extend(segment_data);
                    } else {
                        if optimized_segments.len() + 1 > MAX_SEGMENT_COUNT {
                            // If we exceed the maximum segment count, return the full frame as one segment
                            return full_frame_segment(full_frame_data, frame_width, frame_height);
                        }
                        // Push the current segment if it has enough rows
                        if segment.height as usize >= MIN_SEGMENT_ROWS {
                            optimized_segments.push(segment.clone());
                        }
                        // Start a new segment
                        *segment = Segment {
                            x: 0,
                            y: y as i32,
                            width: frame_width as u32,
                            height: 1,
                            data: segment_data,
                        };
                    }
                } else {
                    // Start the first segment
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
            // If the previous frame is not available, send the entire row
            let segment_data = full_frame_data[start..end].to_vec();
            if let Some(ref mut segment) = current_segment {
                if segment.y + segment.height as i32 == y as i32
                    && segment.width as usize == frame_width
                {
                    segment.height += 1;
                    segment.data.extend(segment_data);
                } else {
                    if optimized_segments.len() + 1 > MAX_SEGMENT_COUNT {
                        // If we exceed the maximum segment count, return the full frame as one segment
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

    // Push the last segment if it exists and has enough rows
    if let Some(segment) = current_segment {
        optimized_segments.push(segment);
    }

    // Note: we do not mutate or copy into the previous-frame buffer here.
    // The caller should call `PrevFrame::update_with_frame(cur_frame_vec)` after
    // sending the frame to rotate buffers and avoid an O(N) copy.

    optimized_segments
}

// fn find_diffs(
//     full_frame_data: &[u8],
//     prev_frame: &[u8],
//     width: usize,
//     height: usize,
//     pixel_bytes: usize,
// ) -> Vec<(usize, usize, usize, usize)> {
//     let mut diffs = Vec::new();

//     for y in 0..height {
//         for x in 0..width {
//             let start = (y * width + x) * pixel_bytes;
//             let end = start + pixel_bytes;
//             if full_frame_data[start..end] != prev_frame[start..end] {
//                 diffs.push((x, y, pixel_bytes, 1));
//             }
//         }
//     }

//     diffs
// }

// fn optimal_diff_groups(
//     diffs: &[(usize, usize, usize, usize)],
//     width: usize,
//     height: usize,
// ) -> Vec<(usize, usize, usize, usize)> {
//     let mut groups = Vec::new();
//     let mut current_group = None;

//     for &(x, y, pixel_bytes, height) in diffs {
//         if let Some((group_x, group_y, group_width, group_height)) = current_group {
//             if x == group_x && y == group_y + group_height {
//                 current_group = Some((group_x, group_y, group_width, group_height + height));
//             } else {
//                 groups.push((group_x, group_y, group_width, group_height));
//                 current_group = Some((x, y, pixel_bytes, height));
//             }
//         } else {
//             current_group = Some((x, y, pixel_bytes, height));
//         }
//     }

//     if let Some((group_x, group_y, group_width, group_height)) = current_group {
//         groups.push((group_x, group_y, group_width, group_height));
//     }

//     groups
// }
