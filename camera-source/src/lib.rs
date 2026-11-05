//! Capture modes and frame layouts shared by the host and virtual camera.
use std::{fmt, str::FromStr, time::Duration};

// Protocol v1 remains compatible with the original 640x480/15 sender.
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
pub const FPS: u64 = 15;
pub const NV12_SIZE: usize = WIDTH * HEIGHT * 3 / 2;
pub const MAX_FRAME_AGE_MS: u64 = 400;
pub const MAX_NV12_SIZE: usize = 1920 * 1080 * 3 / 2;
pub const LEGACY_MODE: VideoMode = VideoMode {
    width: 640,
    height: 480,
    fps: 15,
};
pub const DEFAULT_MODE: VideoMode = VideoMode {
    width: 1280,
    height: 720,
    fps: 30,
};
pub const SUPPORTED_MODES: [VideoMode; 9] = [
    DEFAULT_MODE,
    VideoMode {
        width: 640,
        height: 480,
        fps: 30,
    },
    VideoMode {
        width: 1920,
        height: 1080,
        fps: 30,
    },
    VideoMode {
        width: 1280,
        height: 720,
        fps: 60,
    },
    VideoMode {
        width: 640,
        height: 480,
        fps: 60,
    },
    VideoMode {
        width: 1920,
        height: 1080,
        fps: 60,
    },
    VideoMode {
        width: 1280,
        height: 720,
        fps: 15,
    },
    LEGACY_MODE,
    VideoMode {
        width: 1920,
        height: 1080,
        fps: 15,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoMode {
    width: u16,
    height: u16,
    fps: u16,
}

impl VideoMode {
    pub fn new(width: u16, height: u16, fps: u16) -> Result<Self, &'static str> {
        SUPPORTED_MODES
            .iter()
            .copied()
            .find(|mode| mode.width == width && mode.height == height && mode.fps == fps)
            .ok_or("unsupported video mode; use 640x480, 1280x720 or 1920x1080 at 15, 30 or 60 fps")
    }
    pub const fn width(self) -> usize {
        self.width as usize
    }
    pub const fn height(self) -> usize {
        self.height as usize
    }
    pub const fn fps(self) -> u16 {
        self.fps
    }
    pub const fn nv12_size(self) -> usize {
        self.width() * self.height() * 3 / 2
    }
    pub const fn rgb_size(self) -> usize {
        self.width() * self.height() * 3
    }
    pub fn period(self) -> Duration {
        Duration::from_nanos(1_000_000_000 / u64::from(self.fps))
    }

    pub fn wire(self) -> [u8; 6] {
        let mut bytes = [0; 6];
        bytes[..2].copy_from_slice(&self.width.to_be_bytes());
        bytes[2..4].copy_from_slice(&self.height.to_be_bytes());
        bytes[4..6].copy_from_slice(&self.fps.to_be_bytes());
        bytes
    }

    pub fn from_wire(bytes: [u8; 6]) -> Result<Self, &'static str> {
        Self::new(
            u16::from_be_bytes(bytes[..2].try_into().unwrap()),
            u16::from_be_bytes(bytes[2..4].try_into().unwrap()),
            u16::from_be_bytes(bytes[4..6].try_into().unwrap()),
        )
    }
}

impl fmt::Display for VideoMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}@{}", self.width, self.height, self.fps)
    }
}

impl FromStr for VideoMode {
    type Err = &'static str;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (size, fps) = text.split_once('@').ok_or("expected WIDTHxHEIGHT@FPS")?;
        let (width, height) = size.split_once('x').ok_or("expected WIDTHxHEIGHT@FPS")?;
        Self::new(
            width.parse().map_err(|_| "invalid width")?,
            height.parse().map_err(|_| "invalid height")?,
            fps.parse().map_err(|_| "invalid fps")?,
        )
    }
}

pub mod frame;

#[cfg(all(windows, feature = "com-server"))]
mod media;
#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
pub const CAMERA_CLSID: windows::core::GUID =
    windows::core::GUID::from_u128(0x969b2ae1_0e0a_4dd1_9db2_831d4141f645);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_bounded_and_round_trip() {
        for mode in SUPPORTED_MODES {
            assert_eq!(mode.to_string().parse::<VideoMode>().unwrap(), mode);
            assert_eq!(VideoMode::from_wire(mode.wire()).unwrap(), mode);
            assert!(mode.nv12_size() <= MAX_NV12_SIZE);
        }
        for text in [
            "0x0@30",
            "65535x65535@60",
            "1280x720@0",
            "1280x720@120",
            "720p",
        ] {
            assert!(text.parse::<VideoMode>().is_err());
        }
    }
}
