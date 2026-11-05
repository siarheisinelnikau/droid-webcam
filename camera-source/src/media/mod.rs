//! In-process COM server loaded by Windows Camera Frame Server.
mod activation;
mod source;
mod stream;

use std::{ffi::c_void, ptr};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_POINTER, S_FALSE, S_OK};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::core::{GUID, HRESULT, Interface, Result};

use crate::CAMERA_CLSID;
const STREAM_ID: u32 = 0;
const NULL_GUID: GUID = GUID::zeroed();

#[unsafe(no_mangle)]
unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    unsafe {
        *out = ptr::null_mut();
        if *clsid != CAMERA_CLSID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: windows::Win32::System::Com::IClassFactory = activation::Factory.into();
        HRESULT::from(query_interface(&factory, iid, out))
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    // Frame Server can retain weak COM references and scheduled MF callbacks.
    // Keep the module resident until the service process exits.
    S_FALSE
}

unsafe fn query_interface(
    interface: &impl Interface,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> Result<()> {
    if iid.is_null() || out.is_null() {
        return Err(E_POINTER.into());
    }
    unsafe { interface.query(iid, out).ok() }
}

fn attributes(size: u32) -> Result<IMFAttributes> {
    let mut store = None;
    unsafe { MFCreateAttributes(&mut store, size)? };
    store.ok_or_else(|| E_POINTER.into())
}

fn queue_event(
    queue: &IMFMediaEventQueue,
    event: MF_EVENT_TYPE,
    value: *const PROPVARIANT,
) -> Result<()> {
    unsafe { queue.QueueEventParamVar(event.0 as u32, &NULL_GUID, S_OK, value) }
}

macro_rules! event_generator {
    ($object:ty) => {
        impl IMFMediaEventGenerator_Impl for $object {
            fn GetEvent(
                &self,
                flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
            ) -> Result<IMFMediaEvent> {
                // The queue is itself synchronized. Blocking here must not lock the source.
                unsafe { self.events.GetEvent(flags.0 as u32) }
            }

            fn BeginGetEvent(
                &self,
                callback: Ref<IMFAsyncCallback>,
                state: Ref<IUnknown>,
            ) -> Result<()> {
                unsafe { self.events.BeginGetEvent(callback.as_ref(), state.as_ref()) }
            }

            fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
                unsafe { self.events.EndGetEvent(result.as_ref()) }
            }

            fn QueueEvent(
                &self,
                event: u32,
                guid: *const GUID,
                status: HRESULT,
                value: *const PROPVARIANT,
            ) -> Result<()> {
                unsafe { self.events.QueueEventParamVar(event, guid, status, value) }
            }
        }
    };
}
use event_generator;
