use super::{attributes, query_interface, source::Source};
use std::{ffi::c_void, ptr, slice, sync::Mutex};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_INVALIDARG, E_NOTIMPL, E_POINTER};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    IClassFactory, IClassFactory_Impl, StructuredStorage::PROPVARIANT,
};
use windows::core::{BOOL, GUID, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement};

#[implement(IClassFactory)]
pub(super) struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> Result<()> {
        if out.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *out = ptr::null_mut() };
        if outer.as_ref().is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activation: IMFActivate = Activation {
            attributes: attributes(4)?,
            source: Mutex::new(None),
        }
        .into();
        unsafe { query_interface(&activation, iid, out) }
    }

    fn LockServer(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
}

#[implement(IMFActivate)]
struct Activation {
    attributes: IMFAttributes,
    source: Mutex<Option<IMFMediaSource>>,
}

impl IMFActivate_Impl for Activation_Impl {
    fn ActivateObject(&self, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if out.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *out = ptr::null_mut() };
        let mut source = self.source.lock().unwrap();
        if source.is_none() {
            *source = Some(Source::create(&self.attributes)?);
        }
        unsafe { query_interface(source.as_ref().unwrap(), iid, out) }
    }

    fn ShutdownObject(&self) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn DetachObject(&self) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

impl IMFAttributes_Impl for Activation_Impl {
    fn GetItem(&self, key: *const GUID, value: *mut PROPVARIANT) -> Result<()> {
        unsafe { self.attributes.GetItem(key, Some(value)) }
    }
    fn GetItemType(&self, key: *const GUID) -> Result<MF_ATTRIBUTE_TYPE> {
        unsafe { self.attributes.GetItemType(key) }
    }
    fn CompareItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<BOOL> {
        unsafe { self.attributes.CompareItem(key, value) }
    }
    fn Compare(&self, theirs: Ref<IMFAttributes>, mode: MF_ATTRIBUTES_MATCH_TYPE) -> Result<BOOL> {
        unsafe { self.attributes.Compare(theirs.as_ref(), mode) }
    }
    fn GetUINT32(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.GetUINT32(key) }
    }
    fn GetUINT64(&self, key: *const GUID) -> Result<u64> {
        unsafe { self.attributes.GetUINT64(key) }
    }
    fn GetDouble(&self, key: *const GUID) -> Result<f64> {
        unsafe { self.attributes.GetDouble(key) }
    }
    fn GetGUID(&self, key: *const GUID) -> Result<GUID> {
        unsafe { self.attributes.GetGUID(key) }
    }
    fn GetStringLength(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.GetStringLength(key) }
    }
    fn GetString(&self, key: *const GUID, value: PWSTR, size: u32, length: *mut u32) -> Result<()> {
        if value.is_null() {
            return Err(E_POINTER.into());
        }
        let buffer = unsafe { slice::from_raw_parts_mut(value.0, size as usize) };
        unsafe { self.attributes.GetString(key, buffer, Some(length)) }
    }
    fn GetAllocatedString(
        &self,
        key: *const GUID,
        value: *mut PWSTR,
        length: *mut u32,
    ) -> Result<()> {
        unsafe { self.attributes.GetAllocatedString(key, value, length) }
    }
    fn GetBlobSize(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.GetBlobSize(key) }
    }
    fn GetBlob(&self, key: *const GUID, value: *mut u8, size: u32, length: *mut u32) -> Result<()> {
        if value.is_null() {
            return Err(E_POINTER.into());
        }
        let buffer = unsafe { slice::from_raw_parts_mut(value, size as usize) };
        unsafe { self.attributes.GetBlob(key, buffer, Some(length)) }
    }
    fn GetAllocatedBlob(
        &self,
        key: *const GUID,
        value: *mut *mut u8,
        size: *mut u32,
    ) -> Result<()> {
        unsafe { self.attributes.GetAllocatedBlob(key, value, size) }
    }
    fn GetUnknown(&self, key: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        unsafe {
            (Interface::vtable(&self.attributes).GetUnknown)(
                self.attributes.as_raw(),
                key,
                iid,
                out,
            )
            .ok()
        }
    }
    fn SetItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<()> {
        unsafe { self.attributes.SetItem(key, value) }
    }
    fn DeleteItem(&self, key: *const GUID) -> Result<()> {
        unsafe { self.attributes.DeleteItem(key) }
    }
    fn DeleteAllItems(&self) -> Result<()> {
        unsafe { self.attributes.DeleteAllItems() }
    }
    fn SetUINT32(&self, key: *const GUID, value: u32) -> Result<()> {
        unsafe { self.attributes.SetUINT32(key, value) }
    }
    fn SetUINT64(&self, key: *const GUID, value: u64) -> Result<()> {
        unsafe { self.attributes.SetUINT64(key, value) }
    }
    fn SetDouble(&self, key: *const GUID, value: f64) -> Result<()> {
        unsafe { self.attributes.SetDouble(key, value) }
    }
    fn SetGUID(&self, key: *const GUID, value: *const GUID) -> Result<()> {
        unsafe { self.attributes.SetGUID(key, value) }
    }
    fn SetString(&self, key: *const GUID, value: &PCWSTR) -> Result<()> {
        unsafe { self.attributes.SetString(key, *value) }
    }
    fn SetBlob(&self, key: *const GUID, value: *const u8, size: u32) -> Result<()> {
        let buffer = if size == 0 {
            &[]
        } else if value.is_null() {
            return Err(E_INVALIDARG.into());
        } else {
            unsafe { slice::from_raw_parts(value, size as usize) }
        };
        unsafe { self.attributes.SetBlob(key, buffer) }
    }
    fn SetUnknown(&self, key: *const GUID, value: Ref<IUnknown>) -> Result<()> {
        unsafe { self.attributes.SetUnknown(key, value.as_ref()) }
    }
    fn LockStore(&self) -> Result<()> {
        unsafe { self.attributes.LockStore() }
    }
    fn UnlockStore(&self) -> Result<()> {
        unsafe { self.attributes.UnlockStore() }
    }
    fn GetCount(&self) -> Result<u32> {
        unsafe { self.attributes.GetCount() }
    }
    fn GetItemByIndex(&self, index: u32, key: *mut GUID, value: *mut PROPVARIANT) -> Result<()> {
        unsafe { self.attributes.GetItemByIndex(index, key, Some(value)) }
    }
    fn CopyAllItems(&self, dest: Ref<IMFAttributes>) -> Result<()> {
        unsafe { self.attributes.CopyAllItems(dest.as_ref()) }
    }
}
