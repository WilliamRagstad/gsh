use crate::frame_source::CapturedFrame;
use anyhow::{anyhow, Context, Result};
use std::{
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, watch},
    task::JoinHandle,
};

pub const OUTPUT_WIDTH: u32 = 1920;
pub const OUTPUT_HEIGHT: u32 = 1080;
pub const OUTPUT_FPS: u32 = 60;
pub const GOP_FRAMES: u32 = 30;
const TARGET_BITRATE: &str = "8M";
const OUTPUT_QUEUE_CAPACITY: usize = 2;
const MAX_ACCESS_UNIT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedAccessUnit {
    pub data: Vec<u8>,
    pub is_key_frame: bool,
    pub captured_at: Option<Instant>,
}

#[derive(Default)]
pub struct AnnexBAccessUnitParser {
    buffered: Vec<u8>,
}

impl AnnexBAccessUnitParser {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<EncodedAccessUnit>> {
        self.buffered.extend_from_slice(bytes);
        if self.buffered.len() > MAX_ACCESS_UNIT_BYTES {
            return Err(anyhow!("H.264 access-unit buffer exceeded its limit"));
        }

        let access_unit_delimiters = nal_starts(&self.buffered)
            .into_iter()
            .filter_map(|(start, header)| {
                (header < self.buffered.len() && (self.buffered[header] & 0x1f) == 9)
                    .then_some(start)
            })
            .collect::<Vec<_>>();
        if access_unit_delimiters.len() < 2 {
            return Ok(Vec::new());
        }

        let mut units = Vec::with_capacity(access_unit_delimiters.len() - 1);
        let mut start = 0;
        for boundary in access_unit_delimiters.iter().skip(1) {
            units.push(make_access_unit(self.buffered[start..*boundary].to_vec()));
            start = *boundary;
        }
        self.buffered.drain(..start);
        Ok(units)
    }

    pub fn finish(&mut self) -> Option<EncodedAccessUnit> {
        let data = std::mem::take(&mut self.buffered);
        nal_starts(&data)
            .iter()
            .any(|(_, header)| {
                *header < data.len() && ((data[*header] & 0x1f) == 1 || (data[*header] & 0x1f) == 5)
            })
            .then(|| make_access_unit(data))
    }
}

fn nal_starts(data: &[u8]) -> Vec<(usize, usize)> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        if index + 4 <= data.len() && data[index..index + 4] == [0, 0, 0, 1] {
            starts.push((index, index + 4));
            index += 4;
        } else if data[index..index + 3] == [0, 0, 1] {
            starts.push((index, index + 3));
            index += 3;
        } else {
            index += 1;
        }
    }
    starts
}

fn make_access_unit(data: Vec<u8>) -> EncodedAccessUnit {
    let is_key_frame = nal_starts(&data)
        .iter()
        .any(|(_, header)| *header < data.len() && (data[*header] & 0x1f) == 5);
    EncodedAccessUnit {
        data,
        is_key_frame,
        captured_at: None,
    }
}

pub async fn probe_h264_nvenc() -> bool {
    let mut command = Command::new("ffmpeg");
    command.kill_on_drop(true).args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=1920x1080:r=60,format=rgba",
        "-frames:v",
        "1",
        "-an",
        "-c:v",
        "h264_nvenc",
        "-profile:v",
        "baseline",
        "-preset",
        "p1",
        "-tune",
        "ull",
        "-rc",
        "cbr",
        "-b:v",
        TARGET_BITRATE,
        "-g",
        &GOP_FRAMES.to_string(),
        "-bf",
        "0",
        "-zerolatency",
        "1",
        "-forced-idr",
        "1",
        "-aud",
        "1",
        "-f",
        "null",
        "-",
    ]);
    tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .is_ok_and(|result| result.is_ok_and(|output| output.status.success()))
}

pub struct NvencEncoder {
    pub input: NvencFrameInput,
    pub output: mpsc::Receiver<EncodedAccessUnit>,
    pub process: NvencProcess,
}

pub struct NvencFrameInput {
    latest_frame: watch::Sender<Option<Arc<CapturedFrame>>>,
    frames_written: Arc<std::sync::atomic::AtomicU64>,
}

impl NvencFrameInput {
    pub fn submit_latest(&self, frame: Arc<CapturedFrame>) {
        self.latest_frame.send_replace(Some(frame));
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub struct NvencProcess {
    child: Option<Child>,
    writer_task: Option<JoinHandle<()>>,
    reader_task: Option<JoinHandle<()>>,
}

impl NvencEncoder {
    pub fn spawn(input_width: u32, input_height: u32) -> Result<Self> {
        anyhow::ensure!(
            input_width > 0 && input_height > 0,
            "invalid NVENC input dimensions"
        );
        let fps = OUTPUT_FPS.to_string();
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
            ])
            .arg(format!("{input_width}x{input_height}"))
            .args(["-framerate", &fps, "-i", "pipe:0"]);
        if (input_width, input_height) != (OUTPUT_WIDTH, OUTPUT_HEIGHT) {
            let scale_filter = format!(
                "scale={OUTPUT_WIDTH}:{OUTPUT_HEIGHT}:force_original_aspect_ratio=decrease,pad={OUTPUT_WIDTH}:{OUTPUT_HEIGHT}:(ow-iw)/2:(oh-ih)/2"
            );
            command.args(["-vf", &scale_filter]);
        }
        let mut child = command
            .args([
                "-an",
                "-c:v",
                "h264_nvenc",
                "-profile:v",
                "baseline",
                "-preset",
                "p1",
                "-tune",
                "ull",
                "-rc",
                "cbr",
                "-b:v",
                TARGET_BITRATE,
                "-maxrate",
                TARGET_BITRATE,
                "-bufsize",
                TARGET_BITRATE,
                "-g",
                &GOP_FRAMES.to_string(),
                "-bf",
                "0",
                "-delay",
                "0",
                "-zerolatency",
                "1",
                "-forced-idr",
                "1",
                "-aud",
                "1",
                "-flush_packets",
                "1",
                "-f",
                "h264",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("failed to start FFmpeg NVENC process")?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("FFmpeg stdin pipe unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("FFmpeg stdout pipe unavailable"))?;
        let (latest_frame, frame_receiver) = watch::channel(None::<Arc<CapturedFrame>>);
        let frames_written = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (capture_time_tx, capture_time_rx) = mpsc::channel(OUTPUT_QUEUE_CAPACITY + 2);
        let writer_task = tokio::spawn(write_frames(
            stdin,
            frame_receiver,
            capture_time_tx,
            frames_written.clone(),
            input_width,
            input_height,
        ));
        let (encoded_tx, encoded_frames) = mpsc::channel(OUTPUT_QUEUE_CAPACITY);
        let reader_task = tokio::spawn(read_access_units(stdout, capture_time_rx, encoded_tx));

        Ok(Self {
            input: NvencFrameInput {
                latest_frame,
                frames_written,
            },
            output: encoded_frames,
            process: NvencProcess {
                child: Some(child),
                writer_task: Some(writer_task),
                reader_task: Some(reader_task),
            },
        })
    }

    pub fn into_parts(
        self,
    ) -> (
        NvencFrameInput,
        mpsc::Receiver<EncodedAccessUnit>,
        NvencProcess,
    ) {
        (self.input, self.output, self.process)
    }
}

impl NvencProcess {
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        if let Some(task) = self.writer_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.reader_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(mut child) = self.child.take() {
            child.wait().await.context("waiting for FFmpeg encoder")?;
        }
        Ok(())
    }
}

impl Drop for NvencProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        if let Some(task) = self.writer_task.take() {
            task.abort();
        }
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
    }
}

async fn write_frames(
    mut stdin: ChildStdin,
    mut frames: watch::Receiver<Option<Arc<CapturedFrame>>>,
    capture_time_tx: mpsc::Sender<Instant>,
    frames_written: Arc<std::sync::atomic::AtomicU64>,
    width: u32,
    height: u32,
) {
    while frames.changed().await.is_ok() {
        let Some(frame) = frames.borrow_and_update().clone() else {
            continue;
        };
        if frame.width != width
            || frame.height != height
            || frame.rgba.len() != width as usize * height as usize * 4
        {
            log::warn!("Skipped capture frame with invalid NVENC input dimensions");
            continue;
        }
        if capture_time_tx.send(frame.captured_at).await.is_err() {
            break;
        }
        if let Err(error) = stdin.write_all(&frame.rgba).await {
            log::warn!("FFmpeg NVENC input pipe closed: {error}");
            break;
        }
        frames_written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

async fn read_access_units(
    mut stdout: tokio::process::ChildStdout,
    mut capture_times: mpsc::Receiver<Instant>,
    sender: mpsc::Sender<EncodedAccessUnit>,
) {
    let mut parser = AnnexBAccessUnitParser::default();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = match stdout.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                log::warn!("FFmpeg NVENC output pipe failed: {error}");
                return;
            }
        };
        match parser.push(&buffer[..read]) {
            Ok(mut units) => {
                for mut unit in units.drain(..) {
                    let Some(captured_at) = capture_times.recv().await else {
                        log::warn!("Missing NVENC capture timestamp for encoded access unit");
                        return;
                    };
                    unit.captured_at = Some(captured_at);
                    if sender.send(unit).await.is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                log::error!("Invalid NVENC H.264 stream: {error}");
                return;
            }
        }
    }
    if let Some(mut final_unit) = parser.finish() {
        final_unit.captured_at = capture_times.recv().await;
        let _ = sender.send(final_unit).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libgsh::pacing::FramePacer;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    fn nal(kind: u8, data: u8) -> Vec<u8> {
        vec![0, 0, 0, 1, kind, data]
    }

    #[test]
    fn annex_b_parser_splits_aud_frames_and_marks_idr() {
        let mut stream = Vec::new();
        stream.extend(nal(0x67, 0x01)); // SPS prefix
        stream.extend(nal(0x68, 0x02)); // PPS prefix
        stream.extend(nal(0x09, 0xf0)); // AUD 1
        stream.extend(nal(0x65, 0x03)); // IDR 1
        stream.extend(nal(0x09, 0xf0)); // AUD 2
        stream.extend(nal(0x41, 0x04)); // P frame 2
        stream.extend(nal(0x09, 0xf0)); // AUD 3
        stream.extend(nal(0x41, 0x05)); // P frame 3

        let mut parser = AnnexBAccessUnitParser::default();
        let units = parser.push(&stream).unwrap();
        assert_eq!(units.len(), 2);
        assert!(units[0].is_key_frame);
        assert!(!units[1].is_key_frame);
        assert_eq!(
            parser.finish().unwrap().data,
            nal(0x09, 0xf0)
                .into_iter()
                .chain(nal(0x41, 0x05))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn annex_b_parser_accepts_split_start_codes_and_frames() {
        let stream = [
            nal(0x09, 0xf0),
            nal(0x65, 0x11),
            nal(0x09, 0xf0),
            nal(0x41, 0x22),
        ]
        .concat();
        let mut parser = AnnexBAccessUnitParser::default();
        let split = 10;
        assert!(parser.push(&stream[..split]).unwrap().is_empty());
        let units = parser.push(&stream[split..]).unwrap();
        assert_eq!(units.len(), 1);
        assert!(units[0].is_key_frame);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nvenc_process_encodes_1080p_frames_when_available() {
        if !probe_h264_nvenc().await {
            eprintln!("h264_nvenc unavailable; skipping runtime encoder probe");
            return;
        }
        let encoder = NvencEncoder::spawn(OUTPUT_WIDTH, OUTPUT_HEIGHT).unwrap();
        let (input, mut output, process) = encoder.into_parts();
        let base_pixels = vec![96_u8; OUTPUT_WIDTH as usize * OUTPUT_HEIGHT as usize * 4];
        let mut changed_pixels = base_pixels.clone();
        changed_pixels[..4].copy_from_slice(&[96, 96, 97, 96]);
        let pixel_variants = [
            Arc::<[u8]>::from(base_pixels),
            Arc::<[u8]>::from(changed_pixels),
        ];
        let encoded_timestamps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let reader_timestamps = encoded_timestamps.clone();
        let reader = tokio::spawn(async move {
            while let Some(frame) = output.recv().await {
                assert!(!frame.data.is_empty());
                if let Some(captured_at) = frame.captured_at {
                    reader_timestamps
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(captured_at);
                }
            }
        });
        let mut pacer = FramePacer::new(OUTPUT_FPS).unwrap();
        let mut sequence = 0_u64;
        for index in 0..5 {
            pacer.wait_next().await;
            sequence += 1;
            input.submit_latest(Arc::new(CapturedFrame {
                width: OUTPUT_WIDTH,
                height: OUTPUT_HEIGHT,
                rgba: pixel_variants[index % 2].clone(),
                sequence,
                captured_at: Instant::now(),
            }));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let written_before = input.frames_written();
        let measurement_start = Instant::now();
        for index in 0..OUTPUT_FPS {
            pacer.wait_next().await;
            sequence += 1;
            input.submit_latest(Arc::new(CapturedFrame {
                width: OUTPUT_WIDTH,
                height: OUTPUT_HEIGHT,
                rgba: pixel_variants[index as usize % 2].clone(),
                sequence,
                captured_at: Instant::now(),
            }));
        }
        let measurement_end = Instant::now();
        let send_duration = measurement_end.duration_since(measurement_start);
        let input_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while input.frames_written() < written_before + u64::from(OUTPUT_FPS)
            && tokio::time::Instant::now() < input_deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let frames_written = input.frames_written() - written_before;
        sequence += 1;
        input.submit_latest(Arc::new(CapturedFrame {
            width: OUTPUT_WIDTH,
            height: OUTPUT_HEIGHT,
            rgba: pixel_variants[0].clone(),
            sequence,
            captured_at: Instant::now(),
        }));
        let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let count_in_measurement_window = || {
            encoded_timestamps
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter(|timestamp| {
                    **timestamp >= measurement_start && **timestamp <= measurement_end
                })
                .count()
        };
        while count_in_measurement_window() < 54 && tokio::time::Instant::now() < drain_deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let produced_during_send = count_in_measurement_window();
        let measured_fps = produced_during_send as f64 / send_duration.as_secs_f64();
        println!(
            "NVENC {OUTPUT_WIDTH}x{OUTPUT_HEIGHT}: {frames_written} source frames written, {produced_during_send} access units at {:.3}s ({measured_fps:.1} FPS)",
            send_duration.as_secs_f64()
        );
        assert!(
            produced_during_send >= 54,
            "NVENC encoder fell below 90% of the configured 60 FPS target"
        );

        drop(input);
        reader.abort();
        let _ = reader.await;
        process.shutdown().await.unwrap();
    }
}
