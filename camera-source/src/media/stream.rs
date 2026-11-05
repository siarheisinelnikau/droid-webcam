use super::{NULL_GUID, STREAM_ID, attributes, event_generator, queue_event};
use crate::{DEFAULT_MODE, SUPPORTED_MODES, VideoMode, pipe::Reader};
use std::{collections::VecDeque, ptr, sync::Mutex, time::Instant};
use windows::Win32::Foundation::{E_NOTIMPL, S_OK};
use windows::Win32::Media::KernelStreaming::PINNAME_VIDEO_CAPTURE;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::core::{
    AgileReference, ComObject, GUID, HRESULT, IUnknown, IUnknownImpl, Interface, Ref, Result, Weak,
    implement,
};

const MAX_REQUESTS: usize = 8;

#[implement(IMFMediaStream2)]
pub(super) struct Stream {
    pub(super) descriptor: IMFStreamDescriptor,
    pub(super) attributes: IMFAttributes,
    events: IMFMediaEventQueue,
    source: Mutex<Weak<IMFMediaSource>>,
    state: Mutex<State>,
}

struct State {
    mode: VideoMode,
    running: bool,
    shutdown: bool,
    next: Instant,
    reader: Option<Reader>,
    requests: VecDeque<Option<AgileReference<IUnknown>>>,
    scheduled: Option<u64>,
    generation: u64,
}

impl Stream {
    pub(super) fn create() -> Result<ComObject<Self>> {
        let (descriptor, attributes, events) = unsafe {
            let media_types = SUPPORTED_MODES
                .iter()
                .copied()
                .map(|mode| create_media_type(mode).map(Some))
                .collect::<Result<Vec<_>>>()?;
            let descriptor = MFCreateStreamDescriptor(STREAM_ID, &media_types)?;
            descriptor
                .GetMediaTypeHandler()?
                .SetCurrentMediaType(media_types[0].as_ref())?;
            let attributes = attributes(4)?;
            attributes.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
            attributes.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, STREAM_ID)?;
            attributes.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
            attributes.SetUINT32(
                &MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES,
                MFFrameSourceTypes_Color.0 as u32,
            )?;
            attributes.CopyAllItems(&descriptor)?;
            (descriptor, attributes, MFCreateEventQueue()?)
        };
        Ok(ComObject::new(Self {
            descriptor,
            attributes,
            events,
            source: Mutex::new(Weak::new()),
            state: Mutex::new(State {
                mode: DEFAULT_MODE,
                running: false,
                shutdown: false,
                next: Instant::now(),
                reader: None,
                requests: VecDeque::new(),
                scheduled: None,
                generation: 0,
            }),
        }))
    }

    pub(super) fn attach(&self, source: &IMFMediaSource) -> Result<()> {
        *self.source.lock().unwrap() = source.downgrade()?;
        Ok(())
    }

    pub(super) fn start(&self, time: &PROPVARIANT, mode: VideoMode) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.set_running(false);
        state.mode = mode;
        state.set_running(true);
        queue_event(&self.events, MEStreamStarted, time)
    }

    pub(super) fn stop(&self) -> Result<()> {
        self.state.lock().unwrap().set_running(false);
        queue_event(&self.events, MEStreamStopped, ptr::null())
    }

    pub(super) fn shutdown(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.shutdown {
            return Ok(());
        }
        state.shutdown = true;
        state.set_running(false);
        unsafe { self.events.Shutdown() }
    }
}

impl State {
    fn set_running(&mut self, running: bool) {
        if self.running == running {
            return;
        }
        self.running = running;
        self.next = Instant::now();
        self.reader = running.then(|| Reader::new(self.mode));
        self.requests.clear();
        self.generation = self.generation.wrapping_add(1);
        if let Some(key) = self.scheduled.take() {
            let _ = unsafe { MFCancelWorkItem(key) };
        }
    }

    fn schedule(&mut self, stream: Weak<IMFMediaStream2>) -> Result<()> {
        if self.scheduled.is_some() || self.requests.is_empty() {
            return Ok(());
        }
        let callback: IMFAsyncCallback = SampleWork {
            stream,
            generation: self.generation,
        }
        .into();
        let delay = self
            .next
            .saturating_duration_since(Instant::now())
            .as_millis() as i64;
        let mut key = 0;
        unsafe { MFScheduleWorkItem(&callback, None::<&IUnknown>, -delay.max(1), Some(&mut key))? };
        self.scheduled = Some(key);
        Ok(())
    }
}

event_generator!(Stream_Impl);

impl IMFMediaStream_Impl for Stream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        if self.state.lock().unwrap().shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        self.source
            .lock()
            .unwrap()
            .upgrade()
            .ok_or_else(|| MF_E_SHUTDOWN.into())
    }
    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        if self.state.lock().unwrap().shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(self.descriptor.clone())
    }
    fn RequestSample(&self, token: Ref<IUnknown>) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        if !state.running {
            return Err(MF_E_INVALIDREQUEST.into());
        }
        if state.requests.len() >= MAX_REQUESTS {
            return Err(MF_E_NOTACCEPTING.into());
        }
        let token = token.as_ref().map(AgileReference::new).transpose()?;
        let weak = self.as_interface::<IMFMediaStream2>().downgrade()?;
        state.requests.push_back(token);
        if let Err(error) = state.schedule(weak) {
            state.requests.pop_back();
            return Err(error);
        }
        Ok(())
    }
}

impl IMFMediaStream2_Impl for Stream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        match value {
            MF_STREAM_STATE_RUNNING => state.set_running(true),
            MF_STREAM_STATE_STOPPED => state.set_running(false),
            _ => return Err(MF_E_INVALID_STATE_TRANSITION.into()),
        }
        Ok(())
    }

    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        let state = self.state.lock().unwrap();
        if state.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(if state.running {
            MF_STREAM_STATE_RUNNING
        } else {
            MF_STREAM_STATE_STOPPED
        })
    }
}

#[implement(IMFAsyncCallback)]
struct SampleWork {
    stream: Weak<IMFMediaStream2>,
    generation: u64,
}

impl IMFAsyncCallback_Impl for SampleWork_Impl {
    fn GetParameters(&self, _: *mut u32, _: *mut u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Invoke(&self, _: Ref<IMFAsyncResult>) -> Result<()> {
        let Some(interface) = self.stream.upgrade() else {
            return Ok(());
        };
        let stream = interface.cast_object_ref::<Stream>()?;
        let mut state = stream.state.lock().unwrap();
        if !state.running || state.generation != self.generation {
            return Ok(());
        }
        state.scheduled = None;
        let Some(token) = state.requests.pop_front() else {
            return Ok(());
        };
        let mode = state.mode;
        state.next = (state.next + mode.period()).max(Instant::now());
        let result = emit_sample(
            &stream.events,
            mode,
            state.reader.as_mut().unwrap().latest(),
            token,
        )
        .and_then(|()| state.schedule(self.stream.clone()));
        if let Err(error) = result {
            state.set_running(false);
            unsafe {
                stream.events.QueueEventParamVar(
                    MEError.0 as u32,
                    &NULL_GUID,
                    error.code(),
                    ptr::null(),
                )?;
            }
        }
        Ok(())
    }
}

pub(super) fn mode_from_media(media: &IMFMediaType) -> Result<VideoMode> {
    unsafe {
        if media.GetGUID(&MF_MT_MAJOR_TYPE)? != MFMediaType_Video
            || media.GetGUID(&MF_MT_SUBTYPE)? != MFVideoFormat_NV12
        {
            return Err(MF_E_INVALIDMEDIATYPE.into());
        }
        let size = media.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let rate = media.GetUINT64(&MF_MT_FRAME_RATE)?;
        if rate as u32 != 1
            || (size >> 32) > u16::MAX as u64
            || (size as u32) > u16::MAX as u32
            || (rate >> 32) > u16::MAX as u64
        {
            return Err(MF_E_INVALIDMEDIATYPE.into());
        }
        VideoMode::new((size >> 32) as u16, size as u16, (rate >> 32) as u16)
            .map_err(|_| MF_E_INVALIDMEDIATYPE.into())
    }
}

fn create_media_type(mode: VideoMode) -> Result<IMFMediaType> {
    unsafe {
        let media = MFCreateMediaType()?;
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        media.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (mode.width() as u64) << 32 | mode.height() as u64,
        )?;
        media.SetUINT64(&MF_MT_FRAME_RATE, u64::from(mode.fps()) << 32 | 1)?;
        media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        media.SetUINT32(&MF_MT_DEFAULT_STRIDE, mode.width() as u32)?;
        media.SetUINT32(&MF_MT_SAMPLE_SIZE, mode.nv12_size() as u32)?;
        media.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        media.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        media.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        media.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT601.0 as u32)?;
        Ok(media)
    }
}

fn emit_sample(
    events: &IMFMediaEventQueue,
    mode: VideoMode,
    pixels: &[u8],
    token: Option<AgileReference<IUnknown>>,
) -> Result<()> {
    unsafe {
        let sample = MFCreateSample()?;
        let buffer = MFCreate2DMediaBuffer(
            mode.width() as u32,
            mode.height() as u32,
            MFVideoFormat_NV12.data1,
            false,
        )?;
        buffer.cast::<IMF2DBuffer>()?.ContiguousCopyFrom(pixels)?;
        buffer.SetCurrentLength(mode.nv12_size() as u32)?;
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(MFGetSystemTime())?;
        sample.SetSampleDuration(10_000_000 / i64::from(mode.fps()))?;
        sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
        if let Some(token) = token {
            sample.SetUnknown(&MFSampleExtension_Token, &token.resolve()?)?;
        }
        events.QueueEventParamUnk(MEMediaSample.0 as u32, &NULL_GUID, S_OK, &sample)
    }
}
