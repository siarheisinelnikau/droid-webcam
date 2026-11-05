use super::{NULL_GUID, STREAM_ID, attributes, event_generator, queue_event, stream::Stream};
use std::{ffi::c_void, mem::ManuallyDrop, ptr, sync::Mutex};
use windows::Win32::Foundation::{E_INVALIDARG, E_POINTER, ERROR_SET_NOT_FOUND, S_OK};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, IKsControl_Impl, KSCAMERAPROFILE_Legacy, KSIDENTIFIER,
};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::{
    Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0},
    Variant::{VT_EMPTY, VT_I8},
};
use windows::core::{BOOL, ComObject, GUID, HRESULT, IUnknown, PCWSTR, Ref, Result, implement, w};

#[derive(PartialEq, Eq)]
enum Status {
    Initial,
    Started,
    Stopped,
    Shutdown,
}

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl, IMFSampleAllocatorControl)]
pub(super) struct Source {
    events: IMFMediaEventQueue,
    attributes: IMFAttributes,
    presentation: IMFPresentationDescriptor,
    stream: ComObject<Stream>,
    status: Mutex<Status>,
}

impl Source {
    pub(super) fn create(activation: &IMFAttributes) -> Result<IMFMediaSource> {
        let stream = Stream::create()?;
        unsafe {
            let attributes = attributes(6)?;
            activation.CopyAllItems(&attributes)?;
            attributes.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )?;
            let profile = MFCreateSensorProfile(&KSCAMERAPROFILE_Legacy, 0, PCWSTR::null())?;
            profile.AddProfileFilter(STREAM_ID, w!("((RES==;FRT<=30,1;SUT==))"))?;
            let profiles = MFCreateSensorProfileCollection()?;
            profiles.AddProfile(&profile)?;
            attributes.SetUnknown(&MF_DEVICEMFT_SENSORPROFILE_COLLECTION, &profiles)?;
            let presentation =
                MFCreatePresentationDescriptor(Some(&[Some(stream.descriptor.clone())]))?;
            presentation.SelectStream(0)?;
            let source = ComObject::new(Self {
                events: MFCreateEventQueue()?,
                attributes,
                presentation,
                stream,
                status: Mutex::new(Status::Initial),
            });
            let interface: IMFMediaSource = source.cast()?;
            source.stream.attach(&interface)?;
            Ok(interface)
        }
    }

    fn check(&self) -> Result<()> {
        if *self.status.lock().unwrap() == Status::Shutdown {
            Err(MF_E_SHUTDOWN.into())
        } else {
            Ok(())
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        let _ = self.stream.shutdown();
        let _ = unsafe { self.events.Shutdown() };
    }
}

event_generator!(Source_Impl);

impl IMFMediaSource_Impl for Source_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        self.check()?;
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        self.check()?;
        unsafe { self.presentation.Clone() }
    }

    fn Start(
        &self,
        descriptor: Ref<IMFPresentationDescriptor>,
        format: *const GUID,
        position: *const PROPVARIANT,
    ) -> Result<()> {
        let descriptor = descriptor.as_ref().ok_or(E_INVALIDARG)?;
        let mut status = self.status.lock().unwrap();
        if *status == Status::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        if *status == Status::Started {
            return Err(MF_E_INVALID_STATE_TRANSITION.into());
        }
        unsafe {
            if !format.is_null() && *format != NULL_GUID {
                return Err(MF_E_UNSUPPORTED_TIME_FORMAT.into());
            }
            if position.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let start: &PROPVARIANT_0_0 = &(*position).Anonymous.Anonymous;
            if start.vt != VT_EMPTY && (start.vt != VT_I8 || start.Anonymous.hVal != 0) {
                return Err(MF_E_INVALIDREQUEST.into());
            }
            if descriptor.GetStreamDescriptorCount()? != 1 {
                return Err(E_INVALIDARG.into());
            }
            let mut selected = BOOL(0);
            let mut stream = None;
            descriptor.GetStreamDescriptorByIndex(0, &mut selected, &mut stream)?;
            let stream = stream.ok_or(E_INVALIDARG)?;
            if !selected.as_bool() || stream.GetStreamIdentifier()? != STREAM_ID {
                return Err(E_INVALIDARG.into());
            }
            let media = stream.GetMediaTypeHandler()?.GetCurrentMediaType()?;
            let advertised = self
                .stream
                .descriptor
                .GetMediaTypeHandler()?
                .GetMediaTypeByIndex(0)?;
            let fields = media.IsEqual(&advertised)?;
            if fields
                & (MF_MEDIATYPE_EQUAL_MAJOR_TYPES
                    | MF_MEDIATYPE_EQUAL_FORMAT_TYPES
                    | MF_MEDIATYPE_EQUAL_FORMAT_DATA)
                != (MF_MEDIATYPE_EQUAL_MAJOR_TYPES
                    | MF_MEDIATYPE_EQUAL_FORMAT_TYPES
                    | MF_MEDIATYPE_EQUAL_FORMAT_DATA)
            {
                return Err(MF_E_INVALIDMEDIATYPE.into());
            }

            let event = if *status == Status::Initial {
                MENewStream
            } else {
                MEUpdatedStream
            };
            let stream = self.stream.as_interface::<IMFMediaStream2>();
            self.events
                .QueueEventParamUnk(event.0 as u32, &NULL_GUID, S_OK, &*stream)?;
            let time = event_time();
            self.stream.start(&time)?;
            *status = Status::Started;
            queue_event(&self.events, MESourceStarted, &time)
        }
    }

    fn Stop(&self) -> Result<()> {
        let mut status = self.status.lock().unwrap();
        if *status == Status::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        if *status == Status::Started {
            self.stream.stop()?;
            *status = Status::Stopped;
        }
        queue_event(&self.events, MESourceStopped, &event_time())
    }

    fn Pause(&self) -> Result<()> {
        self.check()?;
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }

    fn Shutdown(&self) -> Result<()> {
        let mut status = self.status.lock().unwrap();
        if *status == Status::Shutdown {
            return Ok(());
        }
        *status = Status::Shutdown;
        let stream = self.stream.shutdown();
        let source = unsafe { self.events.Shutdown() };
        stream.and(source)
    }
}

fn event_time() -> PROPVARIANT {
    PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_I8,
                Anonymous: PROPVARIANT_0_0_0 {
                    hVal: unsafe { MFGetSystemTime() },
                },
                ..Default::default()
            }),
        },
    }
}

impl IMFMediaSourceEx_Impl for Source_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        self.check()?;
        Ok(self.attributes.clone())
    }
    fn GetStreamAttributes(&self, stream: u32) -> Result<IMFAttributes> {
        self.check()?;
        if stream != STREAM_ID {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        Ok(self.stream.attributes.clone())
    }
    fn SetD3DManager(&self, _: Ref<IUnknown>) -> Result<()> {
        self.check()
    }
}

impl IMFGetService_Impl for Source_Impl {
    fn GetService(&self, _: *const GUID, _: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if out.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *out = ptr::null_mut() }
        self.check()?;
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IKsControl_Impl for Source_Impl {
    fn KsProperty(
        &self,
        _: *const KSIDENTIFIER,
        _: u32,
        _: *mut c_void,
        _: u32,
        _: *mut u32,
    ) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND.to_hresult().into())
    }
    fn KsMethod(
        &self,
        _: *const KSIDENTIFIER,
        _: u32,
        _: *mut c_void,
        _: u32,
        _: *mut u32,
    ) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND.to_hresult().into())
    }
    fn KsEvent(
        &self,
        _: *const KSIDENTIFIER,
        _: u32,
        _: *mut c_void,
        _: u32,
        _: *mut u32,
    ) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND.to_hresult().into())
    }
}

impl IMFSampleAllocatorControl_Impl for Source_Impl {
    fn SetDefaultAllocator(&self, stream: u32, _: Ref<IUnknown>) -> Result<()> {
        self.check()?;
        if stream != STREAM_ID {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        Ok(())
    }
    fn GetAllocatorUsage(
        &self,
        stream: u32,
        input: *mut u32,
        usage: *mut MFSampleAllocatorUsage,
    ) -> Result<()> {
        self.check()?;
        if stream != STREAM_ID {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        if input.is_null() || usage.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            *input = STREAM_ID;
            *usage = MFSampleAllocatorUsage_UsesCustomAllocator;
        }
        Ok(())
    }
}
