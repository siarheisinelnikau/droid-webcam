use crate::protocol::Result;
use droid_camera_source::{MAX_FRAME_AGE_MS, VideoMode};
use jpeg_decoder::{CodingProcess, Decoder, PixelFormat};
use jpeg_encoder::{ColorType, Encoder};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn decode_nv12(jpeg: &[u8], mode: VideoMode) -> Result<Vec<u8>> {
    if jpeg.get(..2) != Some(&[0xff, 0xd8]) || !jpeg.ends_with(&[0xff, 0xd9]) {
        return Err("missing JPEG markers".into());
    }
    let mut decoder = Decoder::new(Cursor::new(jpeg));
    decoder.set_max_decoding_buffer_size(mode.rgb_size());
    decoder.read_info()?;
    let info = decoder.info().ok_or("missing JPEG frame header")?;
    if usize::from(info.width) != mode.width()
        || usize::from(info.height) != mode.height()
        || info.pixel_format != PixelFormat::RGB24
        || info.coding_process != CodingProcess::DctSequential
    {
        return Err("JPEG dimensions or format do not match the negotiated mode".into());
    }
    let rgb = decoder.decode()?;
    if rgb.len() != mode.rgb_size() {
        return Err("unexpected decoded frame length".into());
    }
    Ok(rgb_to_nv12(&rgb, mode))
}

fn rgb_to_nv12(rgb: &[u8], mode: VideoMode) -> Vec<u8> {
    let (width, height) = (mode.width(), mode.height());
    let mut nv12 = vec![0; mode.nv12_size()];
    let (y, uv) = nv12.split_at_mut(width * height);
    let (rgb, _) = rgb.as_chunks::<3>();
    for (luma, pixel) in y.iter_mut().zip(rgb) {
        let [r, g, b] = pixel.map(i32::from);
        *luma = (16 + ((66 * r + 129 * g + 25 * b + 128) >> 8)) as u8;
    }
    for row in (0..height).step_by(2) {
        for col in (0..width).step_by(2) {
            let top = row * width + col;
            let bottom = top + width;
            let average = |channel| -> i32 {
                [top, top + 1, bottom, bottom + 1]
                    .iter()
                    .map(|offset| i32::from(rgb[*offset][channel]))
                    .sum::<i32>()
                    / 4
            };
            let (r, g, b) = (average(0), average(1), average(2));
            let index = (row / 2) * width + col;
            uv[index] = (128 + ((-38 * r - 74 * g + 112 * b + 128) >> 8)) as u8;
            uv[index + 1] = (128 + ((112 * r - 94 * g - 18 * b + 128) >> 8)) as u8;
        }
    }
    nv12
}

pub fn synthetic_jpeg(index: usize, mode: VideoMode, quality: u8) -> Result<Vec<u8>> {
    let (width, height) = (mode.width(), mode.height());
    const BARS: [[u8; 3]; 8] = [
        [235, 235, 235],
        [235, 235, 16],
        [16, 235, 235],
        [16, 235, 16],
        [235, 16, 235],
        [235, 16, 16],
        [16, 16, 235],
        [16, 16, 16],
    ];
    let mut rgb = Vec::with_capacity(width * height * 3);
    let marker = index % (width - 32);
    for row in 0..height {
        for col in 0..width {
            let color = if row > height * 3 / 4 && (marker..marker + 32).contains(&col) {
                [255; 3]
            } else {
                BARS[col / (width / BARS.len())]
            };
            rgb.extend_from_slice(&color);
        }
    }
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, quality).encode(&rgb, width as u16, height as u16, ColorType::Rgb)?;
    Ok(jpeg)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Waiting,
    Paired,
    Streaming,
    Stopped,
    Disconnected,
    Expired,
}

#[derive(Clone)]
pub struct Frame {
    pub mode: VideoMode,
    pub captured: Instant,
    pub pixels: Arc<[u8]>,
}

pub struct Feed(Mutex<FeedState>);

struct FeedState {
    state: State,
    mode: Option<VideoMode>,
    frame: Option<Frame>,
}

impl Default for Feed {
    fn default() -> Self {
        Self(Mutex::new(FeedState {
            state: State::Waiting,
            mode: None,
            frame: None,
        }))
    }
}

impl Feed {
    pub fn state(&self) -> State {
        self.0.lock().unwrap().state
    }
    pub fn mode(&self) -> Option<VideoMode> {
        self.0.lock().unwrap().mode
    }

    pub fn set_state(&self, state: State) {
        *self.0.lock().unwrap() = FeedState {
            state,
            mode: None,
            frame: None,
        };
    }

    pub fn configure(&self, mode: VideoMode) {
        *self.0.lock().unwrap() = FeedState {
            state: State::Paired,
            mode: Some(mode),
            frame: None,
        };
    }

    pub fn publish(&self, mode: VideoMode, pixels: Vec<u8>) -> Result<()> {
        if pixels.len() != mode.nv12_size() {
            return Err("incorrect NV12 frame length".into());
        }
        *self.0.lock().unwrap() = FeedState {
            state: State::Streaming,
            mode: Some(mode),
            frame: Some(Frame {
                mode,
                captured: Instant::now(),
                pixels: pixels.into(),
            }),
        };
        Ok(())
    }

    pub fn latest(&self) -> Option<Frame> {
        self.0
            .lock()
            .unwrap()
            .frame
            .as_ref()
            .filter(|frame| frame.captured.elapsed() < Duration::from_millis(MAX_FRAME_AGE_MS))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use droid_camera_source::{DEFAULT_MODE, LEGACY_MODE, SUPPORTED_MODES};

    #[test]
    fn decodes_all_modes_and_rejects_mismatched_resolution() {
        for mode in SUPPORTED_MODES {
            let jpeg = synthetic_jpeg(0, mode, 80).unwrap();
            let pixels = decode_nv12(&jpeg, mode).unwrap();
            assert_eq!(pixels.len(), mode.nv12_size());
            assert!(pixels[40] > pixels[mode.width() - 40]);
            assert!(decode_nv12(&jpeg[..20], mode).is_err());
            let other = if mode.width() == 640 {
                DEFAULT_MODE
            } else {
                LEGACY_MODE
            };
            assert!(decode_nv12(&jpeg, other).is_err());
        }
    }

    #[test]
    fn keeps_only_latest_and_clears_it_on_stop_or_age() {
        let feed = Feed::default();
        feed.configure(DEFAULT_MODE);
        assert_eq!(feed.mode(), Some(DEFAULT_MODE));
        assert!(feed.publish(DEFAULT_MODE, vec![0; 10]).is_err());
        feed.publish(DEFAULT_MODE, vec![1; DEFAULT_MODE.nv12_size()])
            .unwrap();
        feed.publish(DEFAULT_MODE, vec![2; DEFAULT_MODE.nv12_size()])
            .unwrap();
        assert_eq!(feed.latest().unwrap().pixels[0], 2);
        feed.0.lock().unwrap().frame.as_mut().unwrap().captured -= Duration::from_secs(1);
        assert!(feed.latest().is_none());
        feed.publish(DEFAULT_MODE, vec![3; DEFAULT_MODE.nv12_size()])
            .unwrap();
        feed.set_state(State::Stopped);
        assert!(feed.latest().is_none());
        assert_eq!(feed.mode(), None);
        assert_eq!(feed.state(), State::Stopped);
    }
}
