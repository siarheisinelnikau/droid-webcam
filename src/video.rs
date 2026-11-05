use crate::protocol::Result;
use droid_camera_source::{HEIGHT, MAX_FRAME_AGE_MS, NV12_SIZE, WIDTH};
use jpeg_decoder::{CodingProcess, Decoder, PixelFormat};
use jpeg_encoder::{ColorType, Encoder};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn decode_nv12(jpeg: &[u8]) -> Result<Vec<u8>> {
    if jpeg.get(..2) != Some(&[0xff, 0xd8]) || !jpeg.ends_with(&[0xff, 0xd9]) {
        return Err("missing JPEG markers".into());
    }
    let mut decoder = Decoder::new(Cursor::new(jpeg));
    decoder.set_max_decoding_buffer_size(WIDTH * HEIGHT * 3);
    decoder.read_info()?;
    let info = decoder.info().ok_or("missing JPEG frame header")?;
    if usize::from(info.width) != WIDTH
        || usize::from(info.height) != HEIGHT
        || info.pixel_format != PixelFormat::RGB24
        || info.coding_process != CodingProcess::DctSequential
    {
        return Err("expected a 640x480 sequential color JPEG".into());
    }
    let rgb = decoder.decode()?;
    if rgb.len() != WIDTH * HEIGHT * 3 {
        return Err("unexpected decoded frame length".into());
    }
    Ok(rgb_to_nv12(&rgb))
}

fn rgb_to_nv12(rgb: &[u8]) -> Vec<u8> {
    let mut nv12 = vec![0; NV12_SIZE];
    let (y, uv) = nv12.split_at_mut(WIDTH * HEIGHT);
    let (rgb, _) = rgb.as_chunks::<3>();
    for (luma, pixel) in y.iter_mut().zip(rgb) {
        let [r, g, b] = pixel.map(i32::from);
        *luma = (16 + ((66 * r + 129 * g + 25 * b + 128) >> 8)) as u8;
    }
    for row in (0..HEIGHT).step_by(2) {
        for col in (0..WIDTH).step_by(2) {
            let top = row * WIDTH + col;
            let bottom = top + WIDTH;
            let average = |channel| -> i32 {
                [top, top + 1, bottom, bottom + 1]
                    .iter()
                    .map(|offset| i32::from(rgb[*offset][channel]))
                    .sum::<i32>()
                    / 4
            };
            let (r, g, b) = (average(0), average(1), average(2));
            let index = (row / 2) * WIDTH + col;
            uv[index] = (128 + ((-38 * r - 74 * g + 112 * b + 128) >> 8)) as u8;
            uv[index + 1] = (128 + ((112 * r - 94 * g - 18 * b + 128) >> 8)) as u8;
        }
    }
    nv12
}

pub fn synthetic_jpeg(index: usize) -> Result<Vec<u8>> {
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
    let mut rgb = Vec::with_capacity(WIDTH * HEIGHT * 3);
    let marker = index % (WIDTH - 32);
    for row in 0..HEIGHT {
        for col in 0..WIDTH {
            let color = if row > HEIGHT * 3 / 4 && (marker..marker + 32).contains(&col) {
                [255; 3]
            } else {
                BARS[col / (WIDTH / BARS.len())]
            };
            rgb.extend_from_slice(&color);
        }
    }
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, 75).encode(&rgb, WIDTH as u16, HEIGHT as u16, ColorType::Rgb)?;
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
    pub captured: Instant,
    pub pixels: Arc<[u8]>,
}

pub struct Feed(Mutex<(State, Option<Frame>)>);

impl Default for Feed {
    fn default() -> Self {
        Self(Mutex::new((State::Waiting, None)))
    }
}

impl Feed {
    pub fn state(&self) -> State {
        self.0.lock().unwrap().0
    }

    pub fn set_state(&self, state: State) {
        *self.0.lock().unwrap() = (state, None);
    }

    pub fn publish(&self, pixels: Vec<u8>) {
        debug_assert_eq!(pixels.len(), NV12_SIZE);
        *self.0.lock().unwrap() = (
            State::Streaming,
            Some(Frame {
                captured: Instant::now(),
                pixels: pixels.into(),
            }),
        );
    }

    pub fn latest(&self) -> Option<Frame> {
        self.0
            .lock()
            .unwrap()
            .1
            .as_ref()
            .filter(|frame| frame.captured.elapsed() < Duration::from_millis(MAX_FRAME_AGE_MS))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_test_bars_and_rejects_wrong_size() {
        let jpeg = synthetic_jpeg(0).unwrap();
        let pixels = decode_nv12(&jpeg).unwrap();
        assert_eq!(pixels.len(), NV12_SIZE);
        assert!(pixels[40] > pixels[600]);
        assert!(decode_nv12(&jpeg[..20]).is_err());
        let mut wrong_size = Vec::new();
        Encoder::new(&mut wrong_size, 75)
            .encode(&[0; 12], 2, 2, ColorType::Rgb)
            .unwrap();
        assert!(decode_nv12(&wrong_size).is_err());
    }

    #[test]
    fn keeps_only_latest_and_clears_it_on_stop_or_age() {
        let feed = Feed::default();
        feed.publish(vec![1; NV12_SIZE]);
        feed.publish(vec![2; NV12_SIZE]);
        assert_eq!(feed.latest().unwrap().pixels[0], 2);
        feed.0.lock().unwrap().1.as_mut().unwrap().captured -= Duration::from_secs(1);
        assert!(feed.latest().is_none());
        feed.publish(vec![3; NV12_SIZE]);
        feed.set_state(State::Stopped);
        assert!(feed.latest().is_none());
        assert_eq!(feed.state(), State::Stopped);
    }
}
