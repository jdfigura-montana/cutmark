//! WASAPI device listing and event driven audio capture with QPC stamped packets.
//!
//! See the recorder spec: Audio capture and tracks, and Sync and reliability
//! rules, Audio and Device loss.

use std::slice;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, RPC_E_CHANGED_MODE, WAIT_EVENT, WAIT_FAILED};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, STGM_READ};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{GUID, HRESULT, HSTRING};

use crate::{CaptureError, with_context};

/// WAV format tag for IEEE float samples.
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
/// WAV format tag for extensible formats described by a sub format GUID.
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// Sub format GUID for IEEE float inside a WAVEFORMATEXTENSIBLE.
const IEEE_FLOAT_SUBFORMAT: GUID = GUID::from_values(0x0000_0003, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);
/// WASAPI stream flag: deliver packets through an event handle.
const AUDCLNT_STREAMFLAGS_EVENTCALLBACK: u32 = 0x0004_0000;
/// WASAPI stream flag: capture the render mix of an output device.
const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x0002_0000;
/// WASAPI stream flag: let WASAPI convert between the given and the device format.
const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x8000_0000;
/// GetBuffer flag: the buffer holds silence.
const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
/// GetBuffer flag: there was a gap since the last packet.
const AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY: u32 = 0x1;
/// HRESULT for AUDCLNT_E_DEVICE_INVALIDATED.
const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004u32 as i32;
/// HRESULT for AUDCLNT_S_BUFFEREMPTY.
const AUDCLNT_S_BUFFEREMPTY: i32 = 0x0889_0007u32 as i32;
/// Buffer duration for event driven capture: 20 ms in 100 nanosecond units.
const BUFFER_100NS: i64 = 200_000;
/// Bound for the packet queue handed to the consumer.
const CHANNEL_CAPACITY: usize = 256;
/// Event wait timeout in ms, only a safety net; the event drives the loop.
const WAIT_TIMEOUT_MS: u32 = 2000;
/// EDataFlow value for render (output) endpoints.
const E_DATA_FLOW_RENDER: EDataFlow = EDataFlow(0);
/// EDataFlow value for capture (input) endpoints.
const E_DATA_FLOW_CAPTURE: EDataFlow = EDataFlow(1);
/// ERole value for the console default device.
const E_ROLE_CONSOLE: ERole = ERole(0);

/// A Windows event handle, wrapped as an integer so it can move between threads.
#[derive(Clone, Copy)]
struct Event(isize);

impl Event {
    /// The raw handle for a single Windows call.
    fn handle(self) -> HANDLE {
        HANDLE(self.0 as *mut core::ffi::c_void)
    }
}

/// Whether an audio endpoint renders (output) or captures (input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// A render endpoint, for example speakers or a Wave Link mix, captured through loopback.
    Output,
    /// A capture endpoint, for example a microphone.
    Input,
}

/// One audio endpoint as shown in the device picker.
#[derive(Clone, Debug)]
pub struct AudioDevice {
    /// The Windows endpoint ID, the stable identity saved in settings.
    pub id: String,
    /// The friendly name shown in the device picker.
    pub name: String,
    /// Whether this is an output or an input.
    pub kind: DeviceKind,
    /// True when this endpoint is the system default for its kind.
    pub is_default: bool,
    /// Mix format sample rate in Hz.
    pub sample_rate: u32,
    /// Mix format channel count.
    pub channels: u16,
}

/// One packet of interleaved 32 bit float samples from a running capture.
pub struct AudioPacket {
    /// QPC timestamp of the first frame in 100 nanosecond units, comparable with video frames.
    pub qpc_100ns: i64,
    /// Number of frames (sample sets) in the packet.
    pub frames: u32,
    /// Channel count, the same for every packet of one capture.
    pub channels: u16,
    /// Sample rate in Hz, the same for every packet of one capture.
    pub sample_rate: u32,
    /// Interleaved 32 bit float samples, frames times channels values.
    pub samples: Vec<f32>,
    /// True when WASAPI flagged a gap before this packet.
    pub discontinuity: bool,
    /// True when WASAPI flagged the buffer as silence; samples are zeros.
    pub silent: bool,
}

/// Everything the capture thread shares with the AudioCapture handle.
struct ThreadState {
    /// The device to capture.
    device: AudioDevice,
    /// Set to stop the thread.
    stop_flag: Arc<AtomicBool>,
    /// Event that wakes the thread.
    event: Event,
    /// Packet queue; the thread drops packets when it is full.
    sender: SyncSender<AudioPacket>,
    /// One shot channel that reports the setup result.
    ready_sender: mpsc::Sender<Result<(), CaptureError>>,
    /// Total packets produced.
    packets: Arc<AtomicU64>,
    /// Packets dropped because the queue was full.
    overflows: Arc<AtomicU64>,
    /// Set when the endpoint was invalidated.
    device_lost: Arc<AtomicBool>,
}

/// A running WASAPI capture of one audio endpoint on its own thread.
pub struct AudioCapture {
    stop_flag: Arc<AtomicBool>,
    event: Event,
    thread: Option<JoinHandle<()>>,
    receiver: Receiver<AudioPacket>,
    packets: Arc<AtomicU64>,
    overflows: Arc<AtomicU64>,
    device_lost: Arc<AtomicBool>,
}

impl AudioCapture {
    /// Starts event driven capture on the given device and returns the running capture.
    pub fn start(device: &AudioDevice) -> Result<Self, CaptureError> {
        let event = create_event()?;
        let stop_flag = Arc::new(AtomicBool::new(false));
        let packets = Arc::new(AtomicU64::new(0));
        let overflows = Arc::new(AtomicU64::new(0));
        let device_lost = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let state = ThreadState {
            device: device.clone(),
            stop_flag: Arc::clone(&stop_flag),
            event,
            sender,
            ready_sender,
            packets: Arc::clone(&packets),
            overflows: Arc::clone(&overflows),
            device_lost: Arc::clone(&device_lost),
        };
        let thread = thread::Builder::new()
            .name(format!("audio-{}", device.name))
            .spawn(move || capture_thread(&state))
            .map_err(|_| CaptureError::Missing("failed to spawn the audio capture thread"))?;
        let setup = ready_receiver.recv().map_err(|_| CaptureError::Missing("the audio capture thread exited before setup finished"))?;
        setup?;
        Ok(Self { stop_flag, event, thread: Some(thread), receiver, packets, overflows, device_lost })
    }

    /// The packet queue; the capture thread drops packets when it is full.
    pub fn receiver(&self) -> &Receiver<AudioPacket> {
        &self.receiver
    }

    /// How many packets were produced so far, including dropped ones.
    pub fn packets(&self) -> u64 {
        self.packets.load(Ordering::Relaxed)
    }

    /// How many packets were dropped because the queue was full.
    pub fn overflows(&self) -> u64 {
        self.overflows.load(Ordering::Relaxed)
    }

    /// True once the endpoint was invalidated and the thread ended; reconnect comes later.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(Ordering::Relaxed)
    }

    /// Stops the capture and joins its thread; Drop does the same.
    pub fn stop(self) {}
}

#[allow(unsafe_code)]
impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        // SAFETY: event is our live event handle, closed only after the thread is joined.
        unsafe {
            let _ = SetEvent(self.event.handle());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // SAFETY: the thread is joined, so this is the last use of the handle.
        unsafe {
            let _ = CloseHandle(self.event.handle());
        }
    }
}

/// Lists active render and capture endpoints, default output and input first in their groups.
pub fn list_audio_devices() -> Result<Vec<AudioDevice>, CaptureError> {
    let _com = ComGuard::new()?;
    let mut devices = endpoints(DeviceKind::Output)?;
    devices.extend(endpoints(DeviceKind::Input)?);
    Ok(devices)
}

/// Finds a device by endpoint ID, or else by exact friendly name.
pub fn find_device(id: &str, name_fallback: &str) -> Option<AudioDevice> {
    let _com = ComGuard::new().ok()?;
    let devices = list_audio_devices().ok()?;
    devices.iter().find(|d| d.id == id).or_else(|| devices.iter().find(|d| d.name == name_fallback)).cloned()
}

/// Lists one kind of active endpoint with the default endpoint first.
fn endpoints(kind: DeviceKind) -> Result<Vec<AudioDevice>, CaptureError> {
    let enumerator = device_enumerator()?;
    let flow = match kind {
        DeviceKind::Output => E_DATA_FLOW_RENDER,
        DeviceKind::Input => E_DATA_FLOW_CAPTURE,
    };
    let default_id = default_endpoint_id(&enumerator, flow)?.unwrap_or_default();
    let collection = with_context("failed to enumerate audio endpoints", enum_audio_endpoints(&enumerator, flow))?;
    let count = with_context("failed to count audio endpoints", collection_count(&collection))?;
    let mut list = Vec::with_capacity(count as usize);
    for index in 0..count {
        let item = match with_context("failed to read an audio endpoint", collection_item(&collection, index)) {
            Ok(item) => item,
            Err(err) => {
                log::warn!("skipping unreadable audio endpoint: {err}");
                continue;
            }
        };
        match device_info(&item, kind) {
            Ok(mut info) => {
                info.is_default = info.id == default_id;
                list.push(info);
            }
            Err(err) => log::warn!("skipping audio endpoint with unreadable details: {err}"),
        }
    }
    if let Some(pos) = list.iter().position(|d| d.is_default) {
        let default = list.remove(pos);
        list.insert(0, default);
    }
    Ok(list)
}

/// Creates the MMDevice enumerator through COM activation of its class.
fn device_enumerator() -> Result<IMMDeviceEnumerator, CaptureError> {
    with_context("failed to create the MMDevice enumerator", create_enumerator())
}

/// Gets the endpoint ID of the console default endpoint for a data flow.
fn default_endpoint_id(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<Option<String>, CaptureError> {
    let default = with_context("failed to get the default audio endpoint", get_default_endpoint(enumerator, flow))?;
    let id = with_context("failed to read the default endpoint ID", device_id(&default))?;
    Ok(Some(id))
}

/// Collects ID, friendly name and mix format of one endpoint.
fn device_info(mmdev: &IMMDevice, kind: DeviceKind) -> Result<AudioDevice, CaptureError> {
    let id = with_context("failed to read the endpoint ID", device_id(mmdev))?;
    let name = match friendly_name(mmdev) {
        Ok(name) => name,
        Err(err) => {
            log::warn!("no friendly name for an audio endpoint: {err}");
            "(unknown audio device)".to_string()
        }
    };
    let (sample_rate, channels) = mix_format_shape(mmdev)?;
    Ok(AudioDevice { id, name, kind, is_default: false, sample_rate, channels })
}

/// Reads the friendly name from the endpoint property store.
#[allow(unsafe_code)]
fn friendly_name(mmdev: &IMMDevice) -> Result<String, CaptureError> {
    let store = with_context("failed to open the audio device property store", open_property_store(mmdev))?;
    // SAFETY: key is a valid reference; the returned PROPVARIANT is ours and Drop clears it.
    let value = with_context("failed to read the device friendly name property", unsafe { store.GetValue(&PKEY_Device_FriendlyName) })?;
    if value.vt() != VT_LPWSTR {
        return Err(CaptureError::Missing("the friendly name property is not a string"));
    }
    // SAFETY: vt is VT_LPWSTR, so the union holds a NUL terminated wide string pointer.
    let name = unsafe { (*value.Anonymous.Anonymous).Anonymous.pwszVal.display().to_string() };
    Ok(name)
}

/// Reads the channel count and sample rate from the endpoint mix format.
fn mix_format_shape(mmdev: &IMMDevice) -> Result<(u32, u16), CaptureError> {
    let client = with_context("failed to activate an audio endpoint", activate_client(mmdev))?;
    let fmt_ptr = with_context("failed to read the device mix format", get_mix_format(&client))?;
    let mix = read_format(fmt_ptr);
    let shape = (mix.nSamplesPerSec, mix.nChannels);
    free_format(fmt_ptr);
    Ok(shape)
}

/// A session's interfaces and format, all created on the capture thread.
struct Session {
    /// The initialized audio client, kept alive for the whole capture.
    client: IAudioClient,
    /// The capture side of the client.
    capture: IAudioCaptureClient,
    /// Channel count of the delivered packets.
    channels: u16,
    /// Sample rate of the delivered packets.
    sample_rate: u32,
}

/// Runs on the capture thread: COM init, setup, loop, and clean COM shutdown.
fn capture_thread(state: &ThreadState) {
    let owns_com = init_com();
    match open_session(&state.device, state.event) {
        Ok(session) => {
            let _ = state.ready_sender.send(Ok(()));
            if let Err(err) = capture_loop(session, state) {
                log::warn!("audio capture on {} failed: {err}", state.device.name);
            }
        }
        Err(err) => {
            let _ = state.ready_sender.send(Err(err));
        }
    }
    if owns_com {
        uninit_com();
    }
}

/// Initializes COM as MTA and returns true when this thread owns the init.
#[allow(unsafe_code)]
fn init_com() -> bool {
    // SAFETY: CoInitializeEx only sets this thread's COM apartment mode.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    hr.is_ok()
}

/// Releases COM on the calling thread after a successful init.
#[allow(unsafe_code)]
fn uninit_com() {
    // SAFETY: balances a successful CoInitializeEx on this thread.
    unsafe { CoUninitialize() };
}

/// Calls CoInitializeEx for the multithreaded apartment.
#[allow(unsafe_code)]
fn co_initialize_mta() -> HRESULT {
    // SAFETY: CoInitializeEx only sets this thread's COM apartment mode.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
}

/// Holds COM initialization for the calling thread and releases it on drop.
struct ComGuard {
    /// True when this call initialized COM and must balance it on drop.
    owns_com: bool,
}

impl ComGuard {
    /// Initializes COM for this thread, tolerating a thread already in another apartment.
    fn new() -> Result<Self, CaptureError> {
        let hr = co_initialize_mta();
        if hr.is_ok() {
            // S_OK and S_FALSE both make COM ours to uninitialize.
            return Ok(Self { owns_com: true });
        }
        if hr == RPC_E_CHANGED_MODE {
            // The thread already runs COM in another mode, for example the STA UI thread.
            return Ok(Self { owns_com: false });
        }
        Err(CaptureError::Windows { context: "failed to initialize COM for audio device access", source: hr.into() })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owns_com {
            uninit_com();
        }
    }
}

/// Opens and starts the WASAPI session for one device.
fn open_session(device: &AudioDevice, event: Event) -> Result<Session, CaptureError> {
    let enumerator = device_enumerator()?;
    let id = HSTRING::from(device.id.as_str());
    let mmdev = with_context("failed to open the audio endpoint by ID", get_device(&enumerator, &id))?;
    let client = with_context("failed to activate the audio endpoint", activate_client(&mmdev))?;
    let fmt_ptr = with_context("failed to read the device mix format", get_mix_format(&client))?;
    let (sample_rate, channels) = {
        let mix = read_format(fmt_ptr);
        (mix.nSamplesPerSec, mix.nChannels)
    };
    let needs_convert = !is_float_format(read_format(fmt_ptr));
    let initialized = initialize_client(&client, needs_convert, device.kind, sample_rate, channels, fmt_ptr);
    free_format(fmt_ptr);
    with_context("failed to initialize the audio client", initialized)?;
    let capture = with_context("failed to get the WASAPI capture client", get_capture_client(&client))?;
    with_context("failed to set the capture event handle", set_event_handle(&client, event))?;
    with_context("failed to start the audio capture", start_client(&client))?;
    Ok(Session { client, capture, channels, sample_rate })
}

/// Pumps packets until stopped or the device is lost, never blocking on a full queue.
#[allow(unsafe_code)]
fn capture_loop(session: Session, state: &ThreadState) -> Result<(), CaptureError> {
    loop {
        if state.stop_flag.load(Ordering::Relaxed) {
            break;
        }
        if wait_for_event(state.event) == WAIT_FAILED {
            return Err(CaptureError::Missing("the audio capture event wait failed"));
        }
        if state.stop_flag.load(Ordering::Relaxed) {
            break;
        }
        loop {
            let pending = with_context("failed to query the WASAPI packet size", next_packet_size(&session.capture))?;
            if pending == 0 {
                break;
            }
            match read_buffer(&session.capture, session.channels, session.sample_rate) {
                Ok(packet) => {
                    state.packets.fetch_add(1, Ordering::Relaxed);
                    match state.sender.try_send(packet) {
                        Err(TrySendError::Full(_)) => {
                            state.overflows.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(TrySendError::Disconnected(_)) => {}
                        Ok(()) => {}
                    }
                }
                Err(BufferError::Empty) => break,
                Err(BufferError::DeviceLost) => {
                    state.device_lost.store(true, Ordering::Relaxed);
                    log::warn!("audio device lost: {} was invalidated; reconnect comes in a later phase", state.device.name);
                    break;
                }
                Err(BufferError::Failed(err)) => return Err(err),
            }
        }
        if state.device_lost.load(Ordering::Relaxed) {
            break;
        }
    }
    stop_client(&session.client)
}

/// Why a GetBuffer call produced no packet.
enum BufferError {
    /// No data was ready; wait for the next event.
    Empty,
    /// The endpoint was invalidated.
    DeviceLost,
    /// Any other Windows error.
    Failed(CaptureError),
}

/// Calls GetBuffer, copies the samples, and releases the WASAPI buffer.
#[allow(unsafe_code)]
fn read_buffer(capture: &IAudioCaptureClient, channels: u16, sample_rate: u32) -> Result<AudioPacket, BufferError> {
    let mut data: *mut u8 = std::ptr::null_mut();
    let mut frames: u32 = 0;
    let mut flags: u32 = 0;
    let mut qpc_pos: u64 = 0;
    // SAFETY: all out pointers point at locals; WASAPI fills them only on success.
    if let Err(err) = unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc_pos)) } {
        return Err(if err.code().0 == AUDCLNT_S_BUFFEREMPTY {
            BufferError::Empty
        } else if err.code().0 == AUDCLNT_E_DEVICE_INVALIDATED {
            BufferError::DeviceLost
        } else {
            BufferError::Failed(CaptureError::Windows { context: "failed to read the WASAPI capture buffer", source: err })
        });
    }
    let len = frames as usize * channels as usize;
    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT != 0;
    // The buffer is only valid until ReleaseBuffer, so copy before releasing.
    let samples = if silent || data.is_null() {
        vec![0.0; len]
    } else {
        // SAFETY: data points at frames times channels float values in the live WASAPI buffer.
        unsafe { slice::from_raw_parts(data as *const f32, len) }.to_vec()
    };
    // SAFETY: frames is the count GetBuffer just handed out for this buffer.
    if let Err(err) = unsafe { capture.ReleaseBuffer(frames) } {
        log::warn!("failed to release the WASAPI capture buffer: {err}");
    }
    Ok(AudioPacket {
        qpc_100ns: qpc_pos as i64,
        frames,
        channels,
        sample_rate,
        samples,
        discontinuity: flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY != 0,
        silent,
    })
}

/// Stops the audio client, tolerating a device that is already gone.
#[allow(unsafe_code)]
fn stop_client(client: &IAudioClient) -> Result<(), CaptureError> {
    // SAFETY: client is the live session we started; Stop ends packet delivery.
    match audio_client_stop(client) {
        Ok(()) => Ok(()),
        Err(err) if err.code().0 == AUDCLNT_E_DEVICE_INVALIDATED => Ok(()),
        Err(err) => Err(CaptureError::Windows { context: "failed to stop the audio client", source: err }),
    }
}

/// True when a mix format already delivers 32 bit IEEE float samples.
#[allow(unsafe_code)]
fn is_float_format(mix: &WAVEFORMATEX) -> bool {
    if mix.wFormatTag == WAVE_FORMAT_IEEE_FLOAT {
        return true;
    }
    if mix.wFormatTag == WAVE_FORMAT_EXTENSIBLE && mix.cbSize as usize >= std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>() {
        // SAFETY: the format is marked extensible, so the WAVEFORMATEXTENSIBLE tail is present.
        let ext = unsafe { &*(mix as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE) };
        // Copy first: the struct is packed, so its fields must not be referenced.
        let sub_format: GUID = ext.SubFormat;
        return sub_format == IEEE_FLOAT_SUBFORMAT;
    }
    false
}

/// Creates the auto reset event that wakes the capture thread.
#[allow(unsafe_code)]
fn create_event() -> Result<Event, CaptureError> {
    // SAFETY: the event has no security attributes or name; we keep the returned handle.
    let handle = with_context("failed to create the audio capture event", unsafe { CreateEventW(None, false, false, None) })?;
    Ok(Event(handle.0 as isize))
}

/// Waits for the capture event, with a timeout as a stop safety net.
#[allow(unsafe_code)]
fn wait_for_event(event: Event) -> WAIT_EVENT {
    // SAFETY: event is our live handle until the capture thread is joined.
    unsafe { WaitForSingleObject(event.handle(), WAIT_TIMEOUT_MS) }
}

/// Activates the MMDevice enumerator through CoCreateInstance.
#[allow(unsafe_code)]
fn create_enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    // SAFETY: no outer unknown; T is the enumerator interface we want back.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

/// Wraps IMMDeviceEnumerator::GetDevice.
#[allow(unsafe_code)]
fn get_device(enumerator: &IMMDeviceEnumerator, id: &HSTRING) -> windows::core::Result<IMMDevice> {
    // SAFETY: id is a valid HSTRING for the call's lifetime; the result is a new reference.
    unsafe { enumerator.GetDevice(id) }
}

/// Wraps IMMDeviceEnumerator::GetDefaultAudioEndpoint.
#[allow(unsafe_code)]
fn get_default_endpoint(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> windows::core::Result<IMMDevice> {
    // SAFETY: plain COM call with value inputs and an interface out parameter.
    unsafe { enumerator.GetDefaultAudioEndpoint(flow, E_ROLE_CONSOLE) }
}

/// Wraps IMMDeviceEnumerator::EnumAudioEndpoints.
#[allow(unsafe_code)]
fn enum_audio_endpoints(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> windows::core::Result<IMMDeviceCollection> {
    // SAFETY: plain COM call with value inputs and an interface out parameter.
    unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) }
}

/// Wraps IMMDeviceCollection::GetCount.
#[allow(unsafe_code)]
fn collection_count(collection: &IMMDeviceCollection) -> windows::core::Result<u32> {
    // SAFETY: only writes the out parameter we pass.
    unsafe { collection.GetCount() }
}

/// Wraps IMMDeviceCollection::Item.
#[allow(unsafe_code)]
fn collection_item(collection: &IMMDeviceCollection, index: u32) -> windows::core::Result<IMMDevice> {
    // SAFETY: index comes from GetCount and the result is a new owned reference.
    unsafe { collection.Item(index) }
}

/// Wraps IMMDevice::GetId and frees the COM allocated string.
#[allow(unsafe_code)]
fn device_id(mmdev: &IMMDevice) -> windows::core::Result<String> {
    // SAFETY: GetId returns an owned PWSTR; we convert it and free it with CoTaskMemFree.
    let id = unsafe { mmdev.GetId() }?;
    // display reads until NUL and is lossy on invalid UTF-16.
    // SAFETY: id points at a NUL terminated string from GetId.
    let text = unsafe { id.display().to_string() };
    // SAFETY: id came from a COM allocator and must be freed exactly once.
    unsafe { CoTaskMemFree(Some(id.as_ptr().cast())) };
    Ok(text)
}

/// Wraps IMMDevice::OpenPropertyStore.
#[allow(unsafe_code)]
fn open_property_store(mmdev: &IMMDevice) -> windows::core::Result<IPropertyStore> {
    // SAFETY: plain COM call with a read access flag.
    unsafe { mmdev.OpenPropertyStore(STGM_READ) }
}

/// Wraps IMMDevice::Activate for the audio client.
#[allow(unsafe_code)]
fn activate_client(mmdev: &IMMDevice) -> windows::core::Result<IAudioClient> {
    // SAFETY: no activation parameters; the result is a new owned interface reference.
    unsafe { mmdev.Activate(CLSCTX_ALL, None) }
}

/// Wraps IAudioClient::GetMixFormat.
#[allow(unsafe_code)]
fn get_mix_format(client: &IAudioClient) -> windows::core::Result<*mut WAVEFORMATEX> {
    // SAFETY: the result is a COM allocated format we own until free_format.
    unsafe { client.GetMixFormat() }
}

/// Reads a WAVEFORMATEX behind a pointer from GetMixFormat.
#[allow(unsafe_code)]
fn read_format<'a>(fmt: *const WAVEFORMATEX) -> &'a WAVEFORMATEX {
    // SAFETY: the pointer comes from GetMixFormat and stays valid until free_format.
    unsafe { &*fmt }
}

/// Frees a format returned by GetMixFormat.
#[allow(unsafe_code)]
fn free_format(fmt: *const WAVEFORMATEX) {
    // SAFETY: fmt came from a COM allocator and is freed exactly once.
    unsafe { CoTaskMemFree(Some(fmt.cast())) };
}

/// Initializes the audio client: shared mode, event driven, 20 ms, float, loopback for outputs.
#[allow(unsafe_code)]
fn initialize_client(
    client: &IAudioClient,
    needs_convert: bool,
    kind: DeviceKind,
    sample_rate: u32,
    channels: u16,
    mix: *const WAVEFORMATEX,
) -> windows::core::Result<()> {
    let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if kind == DeviceKind::Output {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    if needs_convert {
        flags |= AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
    }
    let float_format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: channels,
        nSamplesPerSec: sample_rate,
        nAvgBytesPerSec: sample_rate * u32::from(channels) * 4,
        nBlockAlign: channels * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let format: *const WAVEFORMATEX = if needs_convert { &float_format } else { mix };
    // SAFETY: format is a valid WAVEFORMATEX for this call, and no session GUID is passed.
    unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_100NS, 0, format, None) }
}

/// Wraps IAudioClient::GetService for the capture client.
#[allow(unsafe_code)]
fn get_capture_client(client: &IAudioClient) -> windows::core::Result<IAudioCaptureClient> {
    // SAFETY: generic COM GetService; the result is a new owned reference.
    unsafe { client.GetService() }
}

/// Wraps IAudioClient::SetEventHandle.
#[allow(unsafe_code)]
fn set_event_handle(client: &IAudioClient, event: Event) -> windows::core::Result<()> {
    // SAFETY: event is our live handle.
    unsafe { client.SetEventHandle(event.handle()) }
}

/// Wraps IAudioClient::Start.
#[allow(unsafe_code)]
fn start_client(client: &IAudioClient) -> windows::core::Result<()> {
    // SAFETY: client is initialized but not yet started.
    unsafe { client.Start() }
}

/// Wraps IAudioCaptureClient::GetNextPacketSize.
#[allow(unsafe_code)]
fn next_packet_size(capture: &IAudioCaptureClient) -> windows::core::Result<u32> {
    // SAFETY: only writes the out parameter we pass.
    unsafe { capture.GetNextPacketSize() }
}

/// Wraps IAudioClient::Stop.
#[allow(unsafe_code)]
fn audio_client_stop(client: &IAudioClient) -> windows::core::Result<()> {
    // SAFETY: client is a live started session.
    unsafe { client.Stop() }
}
