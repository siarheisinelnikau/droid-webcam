//! Local, fixed-size, one-way frame handoff into the Windows Camera Frame Server.
use crate::NV12_SIZE;
#[cfg(feature = "com-server")]
use crate::{HEIGHT, MAX_FRAME_AGE_MS, WIDTH};
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

const PIPE: windows::core::PCWSTR = w!(r"\\.\pipe\DroidWebcam.Frame.969b2ae1.v1");
const HEADER: usize = 8;
const MESSAGE: usize = HEADER + NV12_SIZE;

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
                (2 * MESSAGE) as u32,
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
                message: vec![0; MESSAGE],
            })
        }
    }

    pub fn tick(&mut self, frame: Option<(&[u8], u64)>) -> Result<()> {
        unsafe {
            if !self.connected {
                match ConnectNamedPipe(self.pipe.0, None) {
                    // In PIPE_NOWAIT mode this can mean a disconnected instance is
                    // available again. Only ERROR_PIPE_CONNECTED confirms a client.
                    Ok(()) => return Ok(()),
                    Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => {
                        self.connected = true
                    }
                    Err(error) if error.code() == ERROR_PIPE_LISTENING.to_hresult() => {
                        return Ok(());
                    }
                    Err(error)
                        if error.code()
                            == windows::Win32::Foundation::ERROR_NO_DATA.to_hresult() =>
                    {
                        let _ = DisconnectNamedPipe(self.pipe.0);
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                }
            }
            let Some((pixels, age_ms)) = frame else {
                return Ok(());
            };
            if pixels.len() != NV12_SIZE {
                return Ok(());
            }
            let captured = GetTickCount64().saturating_sub(age_ms);
            self.message[..HEADER].copy_from_slice(&captured.to_le_bytes());
            self.message[HEADER..].copy_from_slice(pixels);
            let mut written = 0;
            // A nonblocking message pipe either writes the entire record or reports zero bytes.
            if WriteFile(self.pipe.0, Some(&self.message), Some(&mut written), None).is_err() {
                let _ = DisconnectNamedPipe(self.pipe.0);
                self.connected = false;
            }
            Ok(())
        }
    }
}

#[cfg(feature = "com-server")]
pub(crate) struct Reader {
    pipe: Option<Handle>,
    message: Vec<u8>,
    blank: Vec<u8>,
    captured: Option<u64>,
}

#[cfg(feature = "com-server")]
impl Reader {
    pub(crate) fn new() -> Self {
        let mut blank = vec![128; NV12_SIZE];
        blank[..WIDTH * HEIGHT].fill(16);
        Self {
            pipe: None,
            message: vec![0; MESSAGE],
            blank,
            captured: None,
        }
    }

    pub(crate) fn latest(&mut self) -> &[u8] {
        self.poll();
        let now = unsafe { GetTickCount64() };
        match self.captured {
            Some(captured) if captured <= now && now - captured < MAX_FRAME_AGE_MS => {
                &self.message[HEADER..]
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
            let Some(pipe) = &self.pipe else { return };
            for _ in 0..2 {
                let mut available = 0;
                let mut message_size = 0;
                let peek = PeekNamedPipe(
                    pipe.0,
                    None,
                    0,
                    None,
                    Some(&mut available),
                    Some(&mut message_size),
                );
                if peek.is_ok() && available == 0 {
                    return;
                }
                if peek.is_err() || message_size as usize != MESSAGE || available < message_size {
                    self.pipe = None;
                    self.captured = None;
                    return;
                }
                let mut count = 0;
                if ReadFile(pipe.0, Some(&mut self.message), Some(&mut count), None).is_err()
                    || count as usize != MESSAGE
                {
                    self.pipe = None;
                    self.captured = None;
                    return;
                }
                self.captured = Some(u64::from_le_bytes(
                    self.message[..HEADER].try_into().unwrap(),
                ));
            }
        }
    }
}
