//! Fixed capture mode shared by the receiver and the Windows media source.
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
pub const FPS: u64 = 15;
pub const NV12_SIZE: usize = WIDTH * HEIGHT * 3 / 2;
pub const MAX_FRAME_AGE_MS: u64 = 400;
