use droid_camera_source::CAMERA_CLSID;
use droid_webcam::protocol::Result;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramFilesX64, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath,
};
use windows::core::{HSTRING, PCWSTR, w};

const SOURCE_DLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/camera-source.dll"));
const DLL_NAME: &str = "droid_camera_source.dll";

struct MediaFoundation;

impl MediaFoundation {
    fn start() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(error);
            }
        }
        Ok(Self)
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

pub struct Camera {
    camera: IMFVirtualCamera,
    _mf: MediaFoundation,
}

impl Camera {
    pub fn start() -> Result<Self> {
        let mf = MediaFoundation::start()?;
        let clsid = HSTRING::from(format!("{{{CAMERA_CLSID:?}}}"));
        let camera = unsafe {
            MFCreateVirtualCamera(
                MFVirtualCameraType_SoftwareCameraSource,
                MFVirtualCameraLifetime_Session,
                MFVirtualCameraAccess_CurrentUser,
                w!("Droid Webcam"),
                &clsid,
                None,
            )?
        };
        let camera = Self { camera, _mf: mf };
        unsafe { camera.camera.Start(None::<&IMFAsyncCallback>) }
            .map_err(|e| format!("could not start the virtual camera ({e}). Check source installation and Windows camera privacy settings."))?;
        Ok(camera)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        if let Err(error) = unsafe { self.camera.Shutdown() } {
            eprintln!("Virtual camera shutdown failed ({error}).");
        }
    }
}

fn install_directory() -> Result<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_ProgramFilesX64, KNOWN_FOLDER_FLAG(0), None)?;
        let directory = PathBuf::from(OsString::from_wide(path.as_wide())).join("Droid Webcam");
        CoTaskMemFree(Some(path.0.cast()));
        Ok(directory)
    }
}

fn class_key() -> HSTRING {
    HSTRING::from(format!(r"SOFTWARE\Classes\CLSID\{{{CAMERA_CLSID:?}}}"))
}

fn ensure_regular(path: &Path, directory: bool) -> Result<()> {
    let meta = path.symlink_metadata()?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(format!("unexpected file type at {}", path.display()).into());
    }
    Ok(())
}

pub fn install() -> Result<()> {
    if SOURCE_DLL.is_empty() {
        return Err("camera-source DLL is missing from this build; use build-windows.ps1".into());
    }
    // Also prevents two setup processes or a running host from touching the installation.
    let _guard = droid_camera_source::pipe::Publisher::open()?;
    let directory = install_directory()?;
    match fs::create_dir(&directory) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    ensure_regular(&directory, true)?;
    let dll = directory.join(DLL_NAME);
    let new_file = match OpenOptions::new().write(true).create_new(true).open(&dll) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(SOURCE_DLL).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(&dll);
                return Err(error.into());
            }
            true
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            ensure_regular(&dll, false)?;
            if fs::read(&dll)? != SOURCE_DLL {
                return Err("a different DLL is installed; run uninstall-source first".into());
            }
            false
        }
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = register(&dll) {
        if new_file {
            let _ = fs::remove_file(&dll);
        }
        return Err(error);
    }
    println!("Installed the COM media source at {}", dll.display());
    Ok(())
}

fn register(dll: &Path) -> Result<()> {
    let key = HSTRING::from(format!(r"{}\InprocServer32", class_key()));
    unsafe {
        let mut handle = HKEY::default();
        let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &key,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_WOW64_64KEY,
            None,
            &mut handle,
            Some(&mut disposition),
        )
        .ok()?;
        let set = |name, value: HSTRING| {
            let bytes = value
                .iter()
                .copied()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            RegSetValueExW(handle, name, None, REG_SZ, Some(&bytes)).ok()
        };
        let result = set(w!("ThreadingModel"), HSTRING::from("Both"))
            .and_then(|()| set(PCWSTR::null(), HSTRING::from(dll.as_os_str())));
        let _ = RegCloseKey(handle);
        if result.is_err() && disposition == REG_CREATED_NEW_KEY {
            let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &class_key());
        }
        Ok(result?)
    }
}

pub fn uninstall() -> Result<()> {
    let _guard = droid_camera_source::pipe::Publisher::open()?;
    let directory = install_directory()?;
    let dll = directory.join(DLL_NAME);
    let removed = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &class_key()) };
    if removed != ERROR_FILE_NOT_FOUND && removed != ERROR_PATH_NOT_FOUND {
        removed.ok()?;
    }
    if !directory.exists() {
        println!("COM media source removed.");
        return Ok(());
    }
    ensure_regular(&directory, true)?;
    match fs::remove_file(&dll) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => {
            return Err(format!("COM registration removed. Reboot to release the DLL, then run uninstall-source again ({error}).").into());
        }
    }
    match fs::remove_dir(&directory) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => (),
        Err(error) => return Err(error.into()),
    }
    println!("COM media source removed.");
    Ok(())
}
