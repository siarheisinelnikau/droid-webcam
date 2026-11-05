//! Fixed capture mode shared by the receiver and the Windows media source.
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
pub const FPS: u64 = 15;
pub const NV12_SIZE: usize = WIDTH * HEIGHT * 3 / 2;
pub const MAX_FRAME_AGE_MS: u64 = 400;

#[cfg(all(windows, feature = "com-server"))]
mod media;
#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
pub const CAMERA_CLSID: windows::core::GUID =
    windows::core::GUID::from_u128(0x969b2ae1_0e0a_4dd1_9db2_831d4141f645);
