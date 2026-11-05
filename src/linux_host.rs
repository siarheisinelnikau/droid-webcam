//! V4L2 output for a v4l2loopback device explicitly created by the user.
use droid_camera_source::{VideoMode, frame::black_frame};
use droid_webcam::{protocol::Result, video::Feed};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::{
        fd::AsRawFd,
        linux::fs::MetadataExt,
        unix::fs::{FileTypeExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::Instant,
};

const VIDEO_OUTPUT: u32 = 2;
const NV12: u32 = u32::from_le_bytes(*b"NV12");
const CAP_VIDEO_OUTPUT: u32 = 0x0000_0002;
const CAP_READWRITE: u32 = 0x0100_0000;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;

#[repr(C)]
#[derive(Default)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PixelFormat {
    width: u32,
    height: u32,
    format: u32,
    field: u32,
    bytes_per_line: u32,
    size_image: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    transfer: u32,
}

#[repr(C)]
union FormatData {
    pixels: PixelFormat,
    raw: [u8; 200],
    alignment: u64,
}
#[repr(C)]
struct Format {
    kind: u32,
    data: FormatData,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Fraction {
    numerator: u32,
    denominator: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct OutputParameters {
    capability: u32,
    mode: u32,
    time_per_frame: Fraction,
    extended_mode: u32,
    write_buffers: u32,
    reserved: [u32; 4],
}
#[repr(C)]
union ParametersData {
    output: OutputParameters,
    raw: [u8; 200],
}
#[repr(C)]
struct Parameters {
    kind: u32,
    data: ParametersData,
}

const fn request(direction: u32, number: u32, size: usize) -> libc::c_ulong {
    ((direction << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | number) as libc::c_ulong
}
const QUERY_CAP: libc::c_ulong = request(2, 0, size_of::<Capability>());
const SET_FORMAT: libc::c_ulong = request(3, 5, size_of::<Format>());
const SET_PARAMETERS: libc::c_ulong = request(3, 22, size_of::<Parameters>());

fn ioctl<T>(file: &File, request: libc::c_ulong, value: &mut T) -> io::Result<()> {
    loop {
        // Every request below uses its matching C-compatible payload structure.
        if unsafe { libc::ioctl(file.as_raw_fd(), request, value as *mut T) } >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn virtual_device(path: &Path) -> Result<u64> {
    let metadata = fs::metadata(path)?;
    if !metadata.file_type().is_char_device() {
        return Err("camera output must be a v4l2loopback character device".into());
    }
    let device = metadata.st_rdev();
    let sysfs = fs::canonicalize(format!(
        "/sys/dev/char/{}:{}",
        libc::major(device),
        libc::minor(device)
    ))?;
    if !sysfs.starts_with("/sys/devices/virtual/video4linux") {
        return Err("camera output must be a virtual V4L2 device".into());
    }
    Ok(device)
}

struct Camera {
    file: File,
    mode: VideoMode,
    black: Vec<u8>,
}

impl Camera {
    fn open(path: &Path, mode: VideoMode) -> Result<Self> {
        let device = virtual_device(path).map_err(|error| {
            format!(
                "could not use {} ({error}). Follow the Ubuntu setup in README.md first",
                path.display()
            )
        })?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        if file.metadata()?.st_rdev() != device {
            return Err("camera device changed while opening".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("another Droid Webcam host is using this device".into());
        }
        let mut capability = Capability::default();
        ioctl(&file, QUERY_CAP, &mut capability)?;
        let driver = String::from_utf8_lossy(&capability.driver);
        let caps = if capability.capabilities & CAP_DEVICE_CAPS != 0 {
            capability.device_caps
        } else {
            capability.capabilities
        };
        if !driver.contains("loopback")
            || caps & (CAP_VIDEO_OUTPUT | CAP_READWRITE) != CAP_VIDEO_OUTPUT | CAP_READWRITE
        {
            return Err("device is not an available v4l2loopback video output".into());
        }
        let mut format = Format {
            kind: VIDEO_OUTPUT,
            data: FormatData { raw: [0; 200] },
        };
        format.data.pixels = PixelFormat {
            width: mode.width() as u32,
            height: mode.height() as u32,
            format: NV12,
            field: 1,
            bytes_per_line: mode.width() as u32,
            size_image: mode.nv12_size() as u32,
            colorspace: 1,
            ycbcr_enc: 1,
            quantization: 2,
            transfer: 1,
            ..Default::default()
        };
        ioctl(&file, SET_FORMAT, &mut format)?;
        let actual = unsafe { format.data.pixels };
        if actual.width != mode.width() as u32
            || actual.height != mode.height() as u32
            || actual.format != NV12
            || actual.bytes_per_line != mode.width() as u32
            || actual.size_image != mode.nv12_size() as u32
        {
            return Err("v4l2loopback changed the requested NV12 layout; close camera clients and clear keep_format before retrying".into());
        }
        let mut parameters = Parameters {
            kind: VIDEO_OUTPUT,
            data: ParametersData { raw: [0; 200] },
        };
        parameters.data.output = OutputParameters {
            time_per_frame: Fraction {
                numerator: 1,
                denominator: u32::from(mode.fps()),
            },
            ..Default::default()
        };
        ioctl(&file, SET_PARAMETERS, &mut parameters)?;
        let rate = unsafe { parameters.data.output.time_per_frame };
        if rate.numerator == 0
            || u64::from(rate.denominator) != u64::from(rate.numerator) * u64::from(mode.fps())
        {
            return Err("v4l2loopback rejected the requested frame rate".into());
        }
        println!("Virtual camera at {} · {mode}", path.display());
        Ok(Self {
            file,
            mode,
            black: black_frame(mode),
        })
    }

    fn write_frame(&mut self, pixels: &[u8]) -> Result<()> {
        if pixels.len() != self.mode.nv12_size() {
            return Err("incorrect V4L2 frame length".into());
        }
        loop {
            match self.file.write(pixels) {
                Ok(count) if count == pixels.len() => return Ok(()),
                Ok(_) => return Err("V4L2 accepted only part of a frame".into()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.file.write(&self.black);
    }
}

pub struct CameraPublisher {
    path: PathBuf,
    camera: Option<Camera>,
    next: Instant,
}

impl CameraPublisher {
    pub fn start(path: &Path) -> Result<Self> {
        virtual_device(path).map_err(|error| format!("could not use {} ({error}). Create the virtual device with scripts/ubuntu-camera.sh first", path.display()))?;
        Ok(Self {
            path: path.to_owned(),
            camera: None,
            next: Instant::now(),
        })
    }

    pub fn tick(&mut self, feed: &Feed) -> Result<()> {
        let Some(mode) = feed.mode() else {
            self.camera = None;
            return Ok(());
        };
        if self
            .camera
            .as_ref()
            .is_none_or(|camera| camera.mode != mode)
        {
            self.camera = None;
            self.camera = Some(Camera::open(&self.path, mode)?);
            self.next = Instant::now();
        }
        if Instant::now() < self.next {
            return Ok(());
        }
        self.next = (self.next + mode.period()).max(Instant::now());
        let camera = self.camera.as_mut().unwrap();
        let frame = feed.latest();
        if let Some(frame) = frame {
            camera.write_frame(&frame.pixels)
        } else {
            // A paired sender may still be opening its camera. Publish neutral video.
            let black = std::mem::take(&mut camera.black);
            let result = camera.write_frame(&black);
            camera.black = black;
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_linux_video_uapi_layout() {
        assert_eq!(size_of::<Capability>(), 104);
        assert_eq!(size_of::<Format>(), 208);
        assert_eq!(std::mem::offset_of!(Format, data), 8);
        assert_eq!(size_of::<Parameters>(), 204);
        assert_eq!(
            (QUERY_CAP, SET_FORMAT, SET_PARAMETERS),
            (0x80685600, 0xc0d05605, 0xc0cc5616)
        );
    }

    #[test]
    fn rejects_regular_files_before_opening_for_output() {
        assert!(virtual_device(Path::new("Cargo.toml")).is_err());
        assert!(Camera::open(Path::new("Cargo.toml"), droid_camera_source::DEFAULT_MODE).is_err());
    }
}
