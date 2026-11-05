//! Local, fixed-size, one-way frame handoff into the Windows Camera Frame Server.
#[cfg(feature = "com-server")]
use crate::{
    MAX_FRAME_AGE_MS,
    frame::{HEADER_SIZE, black_frame, decode_packet, render_nv12},
};
use crate::{
    VideoMode,
    frame::{MAX_PACKET_SIZE, encode_packet},
};
#[cfg(feature = "com-server")]
use windows::Win32::{
    Foundation::GENERIC_READ,
    Storage::FileSystem::{
        CreateFileW, FILE_SHARE_MODE, OPEN_EXISTING, ReadFile, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT,
    },
    System::Pipes::PeekNamedPipe,
};
use windows::Win32::{
    Foundation::{
        CloseHandle, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE, INVALID_HANDLE_VALUE,
        LocalFree,
    },
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, PSECURITY_DESCRIPTOR,
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_OUTBOUND, WriteFile},
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_NOWAIT,
            PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
        },
        SystemInformation::GetTickCount64,
    },
};
use windows::core::{Error, Result, w};

const PIPE: windows::core::PCWSTR = w!(r"\\.\pipe\DroidWebcam.Frame.969b2ae1.v2");

struct Handle(HANDLE);
// An owned kernel handle can be used and closed by any thread in this process.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub struct Publisher {
    pipe: Handle,
    connected: bool,
    message: Vec<u8>,
}

impl Publisher {
    pub fn open() -> Result<Self> {
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w!("D:P(A;;GA;;;OW)(A;;GR;;;LS)(A;;GR;;;SY)"),
                1,
                &mut descriptor,
                None,
            )?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let pipe = CreateNamedPipeW(
                PIPE,
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                (2 * MAX_PACKET_SIZE) as u32,
                0,
                0,
                Some(&attributes),
            );
            let error = (pipe == INVALID_HANDLE_VALUE).then(Error::from_thread);
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(descriptor.0)));
            if let Some(error) = error {
                return Err(error);
            }
            Ok(Self {
                pipe: Handle(pipe),
                connected: false,
                message: Vec::new(),
            })
        }
    }

    pub fn tick(&mut self, frame: Option<(VideoMode, &[u8], u64)>) -> Result<bool> {
        unsafe {
            if !self.connected {
                match ConnectNamedPipe(self.pipe.0, None) {
                    // In PIPE_NOWAIT mode this can mean a disconnected instance is
                    // available again. Only ERROR_PIPE_CONNECTED confirms a client.
                    Ok(()) => return Ok(false),
                    Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => {
                        self.connected = true
                    }
                    Err(error) if error.code() == ERROR_PIPE_LISTENING.to_hresult() => {
                        return Ok(false);
                    }
                    Err(error)
                        if error.code()
                            == windows::Win32::Foundation::ERROR_NO_DATA.to_hresult() =>
                    {
                        let _ = DisconnectNamedPipe(self.pipe.0);
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                }
            }
            let Some((mode, pixels, age_ms)) = frame else {
                return Ok(false);
            };
            let captured = GetTickCount64().saturating_sub(age_ms);
            encode_packet(mode, captured, pixels, &mut self.message)
                .map_err(|_| windows::Win32::Foundation::E_INVALIDARG)?;
            let mut written = 0;
            // A nonblocking message pipe either writes the entire record or reports zero bytes.
            if WriteFile(self.pipe.0, Some(&self.message), Some(&mut written), None).is_err() {
                let _ = DisconnectNamedPipe(self.pipe.0);
                self.connected = false;
                return Ok(false);
            }
            Ok(written as usize == self.message.len())
        }
    }
}

#[cfg(feature = "com-server")]
pub(crate) struct Reader {
    pipe: Option<Handle>,
    mode: VideoMode,
    message: Vec<u8>,
    rendered: Vec<u8>,
    blank: Vec<u8>,
    captured: Option<u64>,
}

#[cfg(feature = "com-server")]
impl Reader {
    pub(crate) fn new(mode: VideoMode) -> Self {
        Self {
            pipe: None,
            mode,
            message: Vec::new(),
            rendered: black_frame(mode),
            blank: black_frame(mode),
            captured: None,
        }
    }

    pub(crate) fn latest(&mut self) -> &[u8] {
        self.poll();
        let now = unsafe { GetTickCount64() };
        match self.captured {
            Some(captured) if captured <= now && now - captured < MAX_FRAME_AGE_MS => {
                &self.rendered
            }
            _ => &self.blank,
        }
    }

    fn poll(&mut self) {
        unsafe {
            if self.pipe.is_none() {
                // A local process impersonating the publisher must not impersonate Frame Server.
                self.pipe = CreateFileW(
                    PIPE,
                    GENERIC_READ.0,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    None,
                )
                .ok()
                .map(Handle);
            }
            let Some(handle) = self.pipe.as_ref().map(|pipe| pipe.0) else {
                return;
            };
            let mut received = false;
            for _ in 0..2 {
                let mut available = 0;
                let mut message_size = 0;
                if PeekNamedPipe(
                    handle,
                    None,
                    0,
                    None,
                    Some(&mut available),
                    Some(&mut message_size),
                )
                .is_err()
                {
                    self.pipe = None;
                    self.captured = None;
                    return;
                }
                if available == 0 {
                    break;
                }
                if !(HEADER_SIZE..=MAX_PACKET_SIZE).contains(&(message_size as usize)) {
                    self.pipe = None;
                    self.captured = None;
                    return;
                }
                if available < message_size {
                    break;
                }
                self.message.resize(message_size as usize, 0);
                let mut count = 0;
                if ReadFile(handle, Some(&mut self.message), Some(&mut count), None).is_err()
                    || count != message_size
                {
                    self.pipe = None;
                    self.captured = None;
                    return;
                }
                received = true;
            }
            if received {
                match decode_packet(&self.message) {
                    Ok((mode, captured, pixels))
                        if render_nv12(pixels, mode, self.mode, &mut self.rendered).is_ok() =>
                    {
                        self.captured = Some(captured);
                    }
                    _ => {
                        self.pipe = None;
                        self.captured = None;
                    }
                }
            }
        }
    }
}
