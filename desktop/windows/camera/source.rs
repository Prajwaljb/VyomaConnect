//! The media source behind "Vyoma Phone Camera", as Windows' own virtual camera sample builds one:
//! an IMFActivate (what the camera service creates from the class) that makes a live IMFMediaSourceEx
//! with one video stream (IMFMediaStream2), both answering IKsControl with "no such property set".
//! The stream offers NV12 at 1280x720, 1920x1080 and 640x360, 30 a second, and hands out a sample
//! for each request, paced by its own thread: the phone's newest picture fitted to the size the app
//! chose, or the placeholder. Samples are plain memory buffers.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER, S_FALSE};
use windows::Win32::Media::KernelStreaming::{IKsControl, IKsControl_Impl, KSIDENTIFIER, PINNAME_VIDEO_CAPTURE};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, IClassFactory, IClassFactory_Impl};
use windows::core::{BOOL, ComObject, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement};

use crate::frame::{self, SECTION_BYTES, Shared};

/// HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND): what a camera says to property sets it doesn't have.
const SET_NOT_FOUND: HRESULT = HRESULT(0x8007_0492_u32 as i32);
const FPS: u32 = 30;
/// The sizes apps can pick, the first the default.
const SIZES: [(u32, u32); 3] = [(1280, 720), (1920, 1080), (640, 360)];
/// The section is looked for again this often while it's missing (the agent may not run yet).
const SECTION_RETRY: Duration = Duration::from_secs(1);

/// An MF object used from the stream's own thread: Media Foundation's objects are free-threaded.
struct Free<T>(T);
// SAFETY: see above; only event queues, samples' tokens and the like go in here.
unsafe impl<T> Send for Free<T> {}
unsafe impl<T> Sync for Free<T> {}

/// Milliseconds since Windows started: the same clock in every session.
pub fn now_ms() -> u64 {
    // SAFETY: no arguments.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

// ---- the shared section

/// The section the agent writes the phone's pictures into, mapped.
pub struct Section {
    handle: windows_sys::Win32::Foundation::HANDLE,
    view: windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS,
    shared: Shared,
}

// SAFETY: the handle and view are only closed in Drop; [Shared] is Send + Sync.
unsafe impl Send for Section {}

impl Section {
    /// The camera's side: made with access for the camera service and signed-in people, or opened
    /// when it's there already (or this process may not make global names: not a service).
    pub fn create() -> Option<Section> {
        use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
        let name = wide(&crate::section_name());
        // SYSTEM, LOCAL SERVICE and administrators all; signed-in (interactive) people read and write.
        let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;LS)(A;;GA;;;BA)(A;;GRGW;;;IU)S:(ML;;NW;;;ME)");
        let mut sd = std::ptr::null_mut();
        // SAFETY: a NUL-terminated SDDL string; the descriptor is freed below.
        let made = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } != 0;
        let attrs = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
        // SAFETY: an anonymous (paging file) section of SECTION_BYTES under our name.
        let handle = unsafe {
            CreateFileMappingW(
                windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
                if made { &attrs } else { std::ptr::null() },
                PAGE_READWRITE,
                0,
                SECTION_BYTES as u32,
                name.as_ptr(),
            )
        };
        if made {
            // SAFETY: allocated by the conversion above.
            unsafe { windows_sys::Win32::Foundation::LocalFree(sd) };
        }
        if handle.is_null() {
            return Section::open();
        }
        Section::map(handle)
    }

    /// The agent's side: the section the camera made, if it's there.
    pub fn open() -> Option<Section> {
        use windows_sys::Win32::System::Memory::{FILE_MAP_READ, FILE_MAP_WRITE, OpenFileMappingW};
        let name = wide(&crate::section_name());
        // SAFETY: a NUL-terminated name.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ | FILE_MAP_WRITE, 0, name.as_ptr()) };
        if handle.is_null() { None } else { Section::map(handle) }
    }

    fn map(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<Section> {
        use windows_sys::Win32::System::Memory::{FILE_MAP_READ, FILE_MAP_WRITE, MapViewOfFile};
        // SAFETY: the whole section, mapped for reading and writing; checked below.
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, SECTION_BYTES) };
        if view.Value.is_null() {
            // SAFETY: our handle, closed once.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return None;
        }
        // SAFETY: a page-aligned view of SECTION_BYTES, alive until Drop.
        let shared = unsafe { Shared::new(view.Value.cast()) };
        Some(Section { handle, view, shared })
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        // SAFETY: our view and handle, released once.
        unsafe {
            windows_sys::Win32::System::Memory::UnmapViewOfFile(self.view);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Pictures for samples: the phone's newest from the section, else the placeholder.
#[derive(Default)]
struct Pictures {
    section: Option<Section>,
    tried: Option<Instant>,
    phone: Vec<u8>,
    placeholder: Option<(usize, usize, Vec<u8>)>,
}

impl Pictures {
    fn fill(&mut self, w: usize, h: usize, out: &mut [u8]) {
        if self.section.is_none() && self.tried.is_none_or(|t| t.elapsed() > SECTION_RETRY) {
            self.tried = Some(Instant::now());
            self.section = Section::create();
        }
        if let Some(section) = &self.section
            && let Some((pw, ph)) = section.shared().read(&mut self.phone, now_ms())
        {
            return frame::fit(&self.phone, pw, ph, w, h, out);
        }
        if self.placeholder.as_ref().is_none_or(|p| (p.0, p.1) != (w, h)) {
            let mut p = vec![0u8; w * h * 3 / 2];
            frame::placeholder(w, h, &mut p);
            self.placeholder = Some((w, h, p));
        }
        if let Some((.., p)) = &self.placeholder {
            out[..p.len()].copy_from_slice(p);
        }
    }
}

// ---- DLL entry points

/// The camera service's way in: the class factory for [crate::CLSID].
///
/// # Safety
/// COM's contract: valid pointers to GUIDs and an out-pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    // SAFETY: checked non-null above.
    unsafe {
        *out = std::ptr::null_mut();
        if *clsid != GUID::from_u128(crate::CLSID) {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, out)
    }
}

/// Never unloaded: the stream's thread may still be finishing.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: checked non-null above.
        unsafe { *object = std::ptr::null_mut() };
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activate: IMFActivate = Activator::new()?.into();
        // SAFETY: as above.
        unsafe { activate.query(iid, object).ok() }
    }

    fn LockServer(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
}

// ---- the activator

#[implement(IMFActivate)]
struct Activator {
    attrs: IMFAttributes,
    source: Mutex<Option<IMFMediaSourceEx>>,
}

impl Activator {
    fn new() -> Result<Activator> {
        Ok(Activator { attrs: attributes()?, source: Mutex::new(None) })
    }
}

fn attributes() -> Result<IMFAttributes> {
    let mut a = None;
    // SAFETY: an out-pointer for the new store.
    unsafe { MFCreateAttributes(&mut a, 8)? };
    a.ok_or_else(|| E_POINTER.into())
}

/// Every IMFAttributes call passed to an inner store, as written in Windows' samples.
macro_rules! pass {
    ($self:ident . $f:ident ( $($arg:expr),* )) => {{
        let a = &$self.attrs;
        // SAFETY: the same arguments COM gave us, to the same method of our own store.
        unsafe { (Interface::vtable(a).$f)(Interface::as_raw(a), $($arg),*).ok() }
    }};
    ($self:ident . $f:ident ( $($arg:expr),* ) -> $t:ty) => {{
        let a = &$self.attrs;
        let mut v: $t = Default::default();
        // SAFETY: as above, with an out-value of our own.
        unsafe { (Interface::vtable(a).$f)(Interface::as_raw(a), $($arg,)* &mut v).ok().map(|()| v) }
    }};
}

fn raw<T: Interface>(r: &Ref<T>) -> *mut c_void {
    r.as_ref().map_or(std::ptr::null_mut(), |i| i.as_raw())
}

impl IMFAttributes_Impl for Activator_Impl {
    fn GetItem(&self, key: *const GUID, value: *mut PROPVARIANT) -> Result<()> {
        pass!(self.GetItem(key, value))
    }
    fn GetItemType(&self, key: *const GUID) -> Result<MF_ATTRIBUTE_TYPE> {
        pass!(self.GetItemType(key) -> MF_ATTRIBUTE_TYPE)
    }
    fn CompareItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<BOOL> {
        pass!(self.CompareItem(key, value) -> BOOL)
    }
    fn Compare(&self, theirs: Ref<IMFAttributes>, how: MF_ATTRIBUTES_MATCH_TYPE) -> Result<BOOL> {
        pass!(self.Compare(raw(&theirs), how) -> BOOL)
    }
    fn GetUINT32(&self, key: *const GUID) -> Result<u32> {
        pass!(self.GetUINT32(key) -> u32)
    }
    fn GetUINT64(&self, key: *const GUID) -> Result<u64> {
        pass!(self.GetUINT64(key) -> u64)
    }
    fn GetDouble(&self, key: *const GUID) -> Result<f64> {
        pass!(self.GetDouble(key) -> f64)
    }
    fn GetGUID(&self, key: *const GUID) -> Result<GUID> {
        pass!(self.GetGUID(key) -> GUID)
    }
    fn GetStringLength(&self, key: *const GUID) -> Result<u32> {
        pass!(self.GetStringLength(key) -> u32)
    }
    fn GetString(&self, key: *const GUID, value: PWSTR, size: u32, length: *mut u32) -> Result<()> {
        pass!(self.GetString(key, value, size, length))
    }
    fn GetAllocatedString(&self, key: *const GUID, value: *mut PWSTR, length: *mut u32) -> Result<()> {
        pass!(self.GetAllocatedString(key, value, length))
    }
    fn GetBlobSize(&self, key: *const GUID) -> Result<u32> {
        pass!(self.GetBlobSize(key) -> u32)
    }
    fn GetBlob(&self, key: *const GUID, buf: *mut u8, size: u32, got: *mut u32) -> Result<()> {
        pass!(self.GetBlob(key, buf, size, got))
    }
    fn GetAllocatedBlob(&self, key: *const GUID, buf: *mut *mut u8, size: *mut u32) -> Result<()> {
        pass!(self.GetAllocatedBlob(key, buf, size))
    }
    fn GetUnknown(&self, key: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        pass!(self.GetUnknown(key, iid, out))
    }
    fn SetItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<()> {
        pass!(self.SetItem(key, value))
    }
    fn DeleteItem(&self, key: *const GUID) -> Result<()> {
        pass!(self.DeleteItem(key))
    }
    fn DeleteAllItems(&self) -> Result<()> {
        pass!(self.DeleteAllItems())
    }
    fn SetUINT32(&self, key: *const GUID, value: u32) -> Result<()> {
        pass!(self.SetUINT32(key, value))
    }
    fn SetUINT64(&self, key: *const GUID, value: u64) -> Result<()> {
        pass!(self.SetUINT64(key, value))
    }
    fn SetDouble(&self, key: *const GUID, value: f64) -> Result<()> {
        pass!(self.SetDouble(key, value))
    }
    fn SetGUID(&self, key: *const GUID, value: *const GUID) -> Result<()> {
        pass!(self.SetGUID(key, value))
    }
    fn SetString(&self, key: *const GUID, value: &PCWSTR) -> Result<()> {
        pass!(self.SetString(key, *value))
    }
    fn SetBlob(&self, key: *const GUID, buf: *const u8, size: u32) -> Result<()> {
        pass!(self.SetBlob(key, buf, size))
    }
    fn SetUnknown(&self, key: *const GUID, value: Ref<IUnknown>) -> Result<()> {
        pass!(self.SetUnknown(key, raw(&value)))
    }
    fn LockStore(&self) -> Result<()> {
        pass!(self.LockStore())
    }
    fn UnlockStore(&self) -> Result<()> {
        pass!(self.UnlockStore())
    }
    fn GetCount(&self) -> Result<u32> {
        pass!(self.GetCount() -> u32)
    }
    fn GetItemByIndex(&self, index: u32, key: *mut GUID, value: *mut PROPVARIANT) -> Result<()> {
        pass!(self.GetItemByIndex(index, key, value))
    }
    fn CopyAllItems(&self, dest: Ref<IMFAttributes>) -> Result<()> {
        pass!(self.CopyAllItems(raw(&dest)))
    }
}

impl IMFActivate_Impl for Activator_Impl {
    fn ActivateObject(&self, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if iid.is_null() || out.is_null() {
            return Err(E_POINTER.into());
        }
        let mut slot = self.source.lock().unwrap();
        if slot.is_none() {
            *slot = Some(make_source()?);
        }
        let source = slot.as_ref().ok_or(windows::core::Error::from(E_POINTER))?;
        // SAFETY: checked non-null above.
        unsafe { source.query(iid, out).ok() }
    }

    fn ShutdownObject(&self) -> Result<()> {
        if let Some(s) = self.source.lock().unwrap().take() {
            // SAFETY: our own source.
            let _ = unsafe { s.Shutdown() };
        }
        Ok(())
    }

    fn DetachObject(&self) -> Result<()> {
        self.source.lock().unwrap().take();
        Ok(())
    }
}

// ---- the source

/// A media source of its own, as the camera service would get one (tests read it through
/// Media Foundation's source reader).
pub fn media_source() -> Result<IMFMediaSource> {
    make_source()?.cast()
}

fn make_source() -> Result<IMFMediaSourceEx> {
    let stream = ComObject::new(Stream::new()?);
    // SAFETY: creating the source's own objects; the stream descriptor is the stream's.
    let (queue, pd, attrs) = unsafe {
        let pd = MFCreatePresentationDescriptor(Some(&[Some(stream.sd.clone())]))?;
        pd.SelectStream(0)?;
        (MFCreateEventQueue()?, pd, attributes()?)
    };
    let source = ComObject::new(Source { queue, pd, attrs, stream: stream.clone(), state: Mutex::new(SourceState::default()) });
    let iface: IMFMediaSourceEx = source.to_interface();
    *stream.source.lock().unwrap() = Some(iface.cast()?);
    Ok(iface)
}

#[derive(Default)]
struct SourceState {
    announced: bool,
    shut: bool,
}

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl)]
struct Source {
    queue: IMFMediaEventQueue,
    pd: IMFPresentationDescriptor,
    attrs: IMFAttributes,
    stream: ComObject<Stream>,
    state: Mutex<SourceState>,
}

impl Source {
    fn live(&self) -> Result<()> {
        if self.state.lock().unwrap().shut { Err(MF_E_SHUTDOWN.into()) } else { Ok(()) }
    }
}

impl IMFMediaEventGenerator_Impl for Source_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        // SAFETY: our own event queue.
        unsafe { self.queue.GetEvent(flags.0) }
    }
    fn BeginGetEvent(&self, callback: Ref<IMFAsyncCallback>, state: Ref<IUnknown>) -> Result<()> {
        // SAFETY: as above.
        unsafe { self.queue.BeginGetEvent(callback.as_ref(), state.as_ref()) }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
        // SAFETY: as above.
        unsafe { self.queue.EndGetEvent(result.as_ref()) }
    }
    fn QueueEvent(&self, kind: u32, ext: *const GUID, status: HRESULT, value: *const PROPVARIANT) -> Result<()> {
        // SAFETY: as above.
        unsafe { self.queue.QueueEventParamVar(kind, ext, status, value) }
    }
}

impl IMFMediaSource_Impl for Source_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        self.live()?;
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        self.live()?;
        // SAFETY: our own descriptor.
        unsafe { self.pd.Clone() }
    }

    fn Start(&self, pd: Ref<IMFPresentationDescriptor>, format: *const GUID, position: *const PROPVARIANT) -> Result<()> {
        self.live()?;
        let pd = pd.ok()?;
        // SAFETY: a GUID pointer the caller passed, read if there.
        if !format.is_null() && unsafe { *format } != GUID::zeroed() {
            return Err(MF_E_UNSUPPORTED_TIME_FORMAT.into());
        }
        let empty = PROPVARIANT::default();
        let position = if position.is_null() { &empty as *const PROPVARIANT } else { position };
        // SAFETY: MF calls on the descriptor we were given and our own objects.
        unsafe {
            let mut selected = BOOL(0);
            let mut sd = None;
            pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd)?;
            if !selected.as_bool() {
                return Err(E_INVALIDARG.into());
            }
            let stream: IMFMediaStream2 = self.stream.to_interface();
            let announced = std::mem::replace(&mut self.state.lock().unwrap().announced, true);
            let kind = if announced { MEUpdatedStream } else { MENewStream };
            self.queue.QueueEventParamUnk(kind.0 as u32, &GUID::zeroed(), HRESULT(0), &stream)?;
            self.stream.start(position)?;
            self.queue.QueueEventParamVar(MESourceStarted.0 as u32, &GUID::zeroed(), HRESULT(0), position)?;
        }
        Ok(())
    }

    fn Stop(&self) -> Result<()> {
        self.live()?;
        self.stream.stop()?;
        // SAFETY: our own queue.
        unsafe { self.queue.QueueEventParamVar(MESourceStopped.0 as u32, &GUID::zeroed(), HRESULT(0), std::ptr::null()) }
    }

    fn Pause(&self) -> Result<()> {
        // A live camera doesn't pause.
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }

    fn Shutdown(&self) -> Result<()> {
        let was = std::mem::replace(&mut self.state.lock().unwrap().shut, true);
        if !was {
            self.stream.shutdown();
            self.stream.source.lock().unwrap().take();
            // SAFETY: our own queue.
            let _ = unsafe { self.queue.Shutdown() };
        }
        Ok(())
    }
}

impl IMFMediaSourceEx_Impl for Source_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        self.live()?;
        Ok(self.attrs.clone())
    }

    fn GetStreamAttributes(&self, id: u32) -> Result<IMFAttributes> {
        self.live()?;
        if id != 0 {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        Ok(self.stream.attrs.clone())
    }

    fn SetD3DManager(&self, _: Ref<IUnknown>) -> Result<()> {
        // Samples are plain memory: the graphics card isn't needed.
        Ok(())
    }
}

impl IMFGetService_Impl for Source_Impl {
    fn GetService(&self, _: *const GUID, _: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if !out.is_null() {
            // SAFETY: checked non-null.
            unsafe { *out = std::ptr::null_mut() };
        }
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IKsControl_Impl for Source_Impl {
    fn KsProperty(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(SET_NOT_FOUND.into())
    }
    fn KsMethod(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(SET_NOT_FOUND.into())
    }
    fn KsEvent(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(SET_NOT_FOUND.into())
    }
}

// ---- the stream

#[derive(Default)]
struct StreamState {
    running: bool,
    shut: bool,
    /// Requests waiting for a sample, each with its token (or none).
    tokens: VecDeque<Option<Free<IUnknown>>>,
    /// The size the app chose (the current media type's).
    size: (u32, u32),
}

struct Paced {
    queue: Free<IMFMediaEventQueue>,
    state: Mutex<StreamState>,
    wake: Condvar,
}

#[implement(IMFMediaStream2, IKsControl)]
struct Stream {
    paced: Arc<Paced>,
    sd: IMFStreamDescriptor,
    attrs: IMFAttributes,
    source: Mutex<Option<IMFMediaSource>>,
}

/// The attributes that make a stream a camera's colour stream for the camera service.
fn device_stream(a: &IMFAttributes) -> Result<()> {
    // SAFETY: setting attributes on a store we own.
    unsafe {
        a.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
        a.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
        a.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
        a.SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;
    }
    Ok(())
}

/// NV12 at [w]×[h], 30 a second, square pixels.
fn media_type(w: u32, h: u32) -> Result<IMFMediaType> {
    // SAFETY: a new media type, filled in.
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((FPS as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        t.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        t.SetUINT32(&MF_MT_DEFAULT_STRIDE, w)?;
        t.SetUINT32(&MF_MT_SAMPLE_SIZE, w * h * 3 / 2)?;
        t.SetUINT32(&MF_MT_AVG_BITRATE, w * h * 12 * FPS)?;
        Ok(t)
    }
}

impl Stream {
    fn new() -> Result<Stream> {
        let types: Vec<Option<IMFMediaType>> = SIZES.iter().map(|&(w, h)| media_type(w, h).map(Some)).collect::<Result<_>>()?;
        // SAFETY: the stream's own descriptor and queue.
        let (sd, queue) = unsafe {
            let sd = MFCreateStreamDescriptor(0, &types)?;
            if let Some(Some(first)) = types.first() {
                sd.GetMediaTypeHandler()?.SetCurrentMediaType(first)?;
            }
            (sd, MFCreateEventQueue()?)
        };
        device_stream(&sd)?;
        let attrs = attributes()?;
        device_stream(&attrs)?;
        let paced = Arc::new(Paced { queue: Free(queue), state: Mutex::new(StreamState { size: SIZES[0], ..Default::default() }), wake: Condvar::new() });
        let worker = paced.clone();
        std::thread::Builder::new().name("vyoma-camera".into()).spawn(move || serve(worker)).map_err(|_| windows::core::Error::from(E_POINTER))?;
        Ok(Stream { paced, sd, attrs, source: Mutex::new(None) })
    }

    /// The size the app chose, from the stream descriptor's current type.
    fn chosen_size(&self) -> (u32, u32) {
        // SAFETY: reads of our own descriptor's handler.
        unsafe {
            self.sd
                .GetMediaTypeHandler()
                .and_then(|h| h.GetCurrentMediaType())
                .and_then(|t| t.GetUINT64(&MF_MT_FRAME_SIZE))
                .map(|s| ((s >> 32) as u32, s as u32))
                .ok()
                .filter(|&(w, h)| w >= 2 && h >= 2 && w as usize <= frame::MAX_SIDE && h as usize <= frame::MAX_SIDE)
                .unwrap_or(SIZES[0])
        }
    }

    fn start(&self, position: *const PROPVARIANT) -> Result<()> {
        let size = self.chosen_size();
        {
            let mut s = self.paced.state.lock().unwrap();
            if s.shut {
                return Err(MF_E_SHUTDOWN.into());
            }
            (s.running, s.size) = (true, size);
        }
        self.paced.wake.notify_all();
        // SAFETY: our own queue.
        unsafe { self.paced.queue.0.QueueEventParamVar(MEStreamStarted.0 as u32, &GUID::zeroed(), HRESULT(0), position) }
    }

    fn stop(&self) -> Result<()> {
        {
            let mut s = self.paced.state.lock().unwrap();
            s.running = false;
            s.tokens.clear();
        }
        // SAFETY: our own queue.
        unsafe { self.paced.queue.0.QueueEventParamVar(MEStreamStopped.0 as u32, &GUID::zeroed(), HRESULT(0), std::ptr::null()) }
    }

    fn shutdown(&self) {
        {
            let mut s = self.paced.state.lock().unwrap();
            (s.shut, s.running) = (true, false);
            s.tokens.clear();
        }
        self.paced.wake.notify_all();
        // SAFETY: our own queue.
        let _ = unsafe { self.paced.queue.0.Shutdown() };
    }
}

/// The stream's thread: a sample for each request, at most [FPS] a second.
fn serve(paced: Arc<Paced>) {
    // SAFETY: this thread joins the multithreaded apartment for the MF calls.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let period = Duration::from_nanos(1_000_000_000 / FPS as u64);
    let mut pictures = Pictures::default();
    let mut next = Instant::now();
    loop {
        let (token, size) = {
            let mut s = paced.state.lock().unwrap();
            loop {
                if s.shut {
                    return;
                }
                if s.running && !s.tokens.is_empty() {
                    break;
                }
                s = paced.wake.wait(s).unwrap();
            }
            (s.tokens.pop_front().flatten(), s.size)
        };
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        next = (next + period).max(Instant::now());
        if let Ok(sample) = sample(&mut pictures, size, token.as_ref().map(|t| &t.0)) {
            // SAFETY: our own queue; it takes its own reference to the sample.
            let _ = unsafe { paced.queue.0.QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), HRESULT(0), &sample) };
        }
    }
}

fn sample(pictures: &mut Pictures, (w, h): (u32, u32), token: Option<&IUnknown>) -> Result<IMFSample> {
    let len = w * h * 3 / 2;
    // SAFETY: a memory buffer of [len] bytes, locked while it's written.
    unsafe {
        let buffer = MFCreateMemoryBuffer(len)?;
        let mut data = std::ptr::null_mut();
        buffer.Lock(&mut data, None, None)?;
        pictures.fill(w as usize, h as usize, std::slice::from_raw_parts_mut(data, len as usize));
        buffer.Unlock()?;
        buffer.SetCurrentLength(len)?;
        let s = MFCreateSample()?;
        s.AddBuffer(&buffer)?;
        s.SetSampleTime(MFGetSystemTime())?;
        s.SetSampleDuration(10_000_000 / FPS as i64)?;
        if let Some(t) = token {
            s.SetUnknown(&MFSampleExtension_Token, t)?;
        }
        Ok(s)
    }
}

impl IMFMediaEventGenerator_Impl for Stream_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        // SAFETY: our own event queue.
        unsafe { self.paced.queue.0.GetEvent(flags.0) }
    }
    fn BeginGetEvent(&self, callback: Ref<IMFAsyncCallback>, state: Ref<IUnknown>) -> Result<()> {
        // SAFETY: as above.
        unsafe { self.paced.queue.0.BeginGetEvent(callback.as_ref(), state.as_ref()) }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
        // SAFETY: as above.
        unsafe { self.paced.queue.0.EndGetEvent(result.as_ref()) }
    }
    fn QueueEvent(&self, kind: u32, ext: *const GUID, status: HRESULT, value: *const PROPVARIANT) -> Result<()> {
        // SAFETY: as above.
        unsafe { self.paced.queue.0.QueueEventParamVar(kind, ext, status, value) }
    }
}

impl IMFMediaStream_Impl for Stream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        self.source.lock().unwrap().clone().ok_or_else(|| MF_E_SHUTDOWN.into())
    }

    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        if self.paced.state.lock().unwrap().shut {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(self.sd.clone())
    }

    fn RequestSample(&self, token: Ref<IUnknown>) -> Result<()> {
        {
            let mut s = self.paced.state.lock().unwrap();
            if s.shut {
                return Err(MF_E_SHUTDOWN.into());
            }
            if !s.running {
                return Err(MF_E_INVALIDREQUEST.into());
            }
            s.tokens.push_back(token.cloned().map(Free));
        }
        self.paced.wake.notify_all();
        Ok(())
    }
}

impl IMFMediaStream2_Impl for Stream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> Result<()> {
        let mut s = self.paced.state.lock().unwrap();
        if s.shut {
            return Err(MF_E_SHUTDOWN.into());
        }
        s.running = value == MF_STREAM_STATE_RUNNING;
        if !s.running {
            s.tokens.clear();
        }
        drop(s);
        self.paced.wake.notify_all();
        Ok(())
    }

    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        let s = self.paced.state.lock().unwrap();
        Ok(if s.running { MF_STREAM_STATE_RUNNING } else { MF_STREAM_STATE_STOPPED })
    }
}

impl IKsControl_Impl for Stream_Impl {
    fn KsProperty(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(SET_NOT_FOUND.into())
    }
    fn KsMethod(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(SET_NOT_FOUND.into())
    }
    fn KsEvent(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(SET_NOT_FOUND.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The camera service's way: the DLL's class object, the activator (attributes kept), the source.
    #[test]
    fn the_camera_service_gets_a_source_from_the_class() {
        // SAFETY: COM calls through the DLL's own entry point, as the camera service makes them.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap();
            let mut factory: *mut c_void = std::ptr::null_mut();
            let other = GUID::from_u128(1);
            assert_eq!(DllGetClassObject(&other, &IClassFactory::IID, &mut factory), CLASS_E_CLASSNOTAVAILABLE);
            assert_eq!(DllGetClassObject(&GUID::from_u128(crate::CLSID), &IClassFactory::IID, &mut factory), HRESULT(0));
            let factory = IClassFactory::from_raw(factory);
            let activate: IMFActivate = factory.CreateInstance(None).unwrap();
            activate.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 7).unwrap();
            assert_eq!(activate.GetUINT32(&MF_DEVICESTREAM_STREAM_ID).unwrap(), 7, "the activator keeps what it's given");
            assert_eq!(activate.GetCount().unwrap(), 1);
            let source: IMFMediaSourceEx = activate.ActivateObject().unwrap();
            let again: IMFMediaSource = activate.ActivateObject().unwrap();
            assert_eq!(source.cast::<IUnknown>().unwrap(), again.cast::<IUnknown>().unwrap(), "one source per activator");
            let pd = source.CreatePresentationDescriptor().unwrap();
            assert_eq!(pd.GetStreamDescriptorCount().unwrap(), 1);
            let attrs = source.GetStreamAttributes(0).unwrap();
            assert_eq!(attrs.GetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY).unwrap(), PINNAME_VIDEO_CAPTURE);
            assert!(source.GetStreamAttributes(1).is_err());
            assert!(source.cast::<IKsControl>().is_ok(), "a camera answers IKsControl");
            assert!(source.Pause().is_err(), "a live camera doesn't pause");
            activate.ShutdownObject().unwrap();
            assert_eq!(source.GetCharacteristics().unwrap_err().code(), MF_E_SHUTDOWN);
        }
    }

    /// The source as an app sees it, through Media Foundation's source reader: frames of the
    /// default size come out, the placeholder first, then the phone's picture once the agent
    /// writes one into the section.
    #[test]
    fn apps_read_the_placeholder_then_the_phone() {
        // SAFETY: COM and MF started on this thread; the reader and section are dropped before the end.
        unsafe {
            std::env::set_var("VYOMA_CAMERA_SECTION", "Local\\VyomaCameraTest");
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap();
            let source = media_source().unwrap();
            assert_eq!(source.GetCharacteristics().unwrap() & MFMEDIASOURCE_IS_LIVE.0 as u32, 1);
            let reader = MFCreateSourceReaderFromMediaSource(&source, None).unwrap();
            let read = |reader: &IMFSourceReader| -> Vec<u8> {
                for _ in 0..20 {
                    let (mut flags, mut sample) = (0u32, None);
                    reader.ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, 0, None, Some(&mut flags), None, Some(&mut sample)).unwrap();
                    if let Some(s) = sample {
                        let buffer = s.ConvertToContiguousBuffer().unwrap();
                        let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
                        buffer.Lock(&mut data, None, Some(&mut len)).unwrap();
                        let bytes = std::slice::from_raw_parts(data, len as usize).to_vec();
                        buffer.Unlock().unwrap();
                        return bytes;
                    }
                }
                panic!("no sample");
            };
            let first = read(&reader);
            assert_eq!(first.len(), 1280 * 720 * 3 / 2);
            assert_eq!(first[360 * 1280 + 640], 235, "the placeholder's orb");
            // The agent's side: a grey phone picture.
            let agent = Section::open().expect("the camera made the section");
            let grey = vec![150u8; 640 * 360 * 3 / 2];
            agent.shared().set_live(true);
            agent.shared().write(640, 360, &grey, now_ms());
            let mut phone = read(&reader);
            for _ in 0..5 {
                if phone[0] == 150 {
                    break;
                }
                agent.shared().write(640, 360, &grey, now_ms());
                phone = read(&reader);
            }
            assert_eq!(phone[0], 150, "the phone's picture, scaled up");
            assert_eq!(phone[1280 * 720 - 1], 150);
            agent.shared().set_live(false);
            drop(reader);
            let _ = source.Shutdown();
        }
    }
}
