//! Per-app audio capture through WASAPI process loopback, next to the device
//! loopback in `audio`.
//!
//! Each target process gets its own stream. All of them wake one capture thread
//! through one event, and several streams are mixed by their QPC timestamps.
//!
//! See the per-app audio spec: Capture.

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, E_NOINTERFACE, E_POINTER, S_OK, WAIT_FAILED};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Vtbl, IAudioCaptureClient, IAudioClient, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Threading::{SetEvent, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface};

use crate::app_audio::{APP_CHANNELS, APP_SAMPLE_RATE, AppTarget, AudioPacket, LoopbackMode, Mixer, top_level_processes};
use crate::app_list::{ProcessHandle, fill_creation_times, process_table};
use crate::audio::{
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER_100NS, BufferError, CHANNEL_CAPACITY, Event,
    WAVE_FORMAT_IEEE_FLOAT, create_event, get_capture_client, init_com, next_packet_size, read_buffer, set_event_handle, start_client, stop_client, uninit_com,
};
use crate::{CaptureError, qpc_now_100ns, with_context};

/// The longest wait between two looks at the streams and the target processes.
const POLL_MS: u32 = 20;
/// How long Windows gets to activate a process loopback client.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the process table is read for targets that cannot be waited on.
const TABLE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// A packet timestamp further than this from now is not believed: 5 s.
const MAX_STAMP_ERROR_100NS: i64 = 50_000_000;
/// The IID of IAgileObject, the marker for objects any thread may call.
const IID_AGILE_OBJECT: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);

/// A running capture of one app's sound, or of everything but one app, on its
/// own thread. Packets are 48 kHz stereo float.
pub struct AppLoopback {
    stop_flag: Arc<AtomicBool>,
    event: Event,
    thread: Option<JoinHandle<()>>,
    receiver: Receiver<AudioPacket>,
    packets: Arc<AtomicU64>,
    overflows: Arc<AtomicU64>,
    lost: Arc<AtomicBool>,
    excluded_instances: usize,
    total_instances: usize,
}

/// Everything the capture thread shares with the AppLoopback handle.
struct ThreadState {
    /// The executable, for thread and log names.
    exe: String,
    /// The processes the streams target.
    pids: Vec<u32>,
    /// Whether the streams include or exclude their process trees.
    mode: LoopbackMode,
    /// Set to stop the thread.
    stop_flag: Arc<AtomicBool>,
    /// The event every stream signals; also wakes the thread to stop it.
    event: Event,
    /// Packet queue; the thread drops packets when it is full.
    sender: SyncSender<AudioPacket>,
    /// One shot channel that reports the setup result.
    ready_sender: mpsc::Sender<Result<(), CaptureError>>,
    /// Total packets produced.
    packets: Arc<AtomicU64>,
    /// Packets dropped because the queue was full.
    overflows: Arc<AtomicU64>,
    /// Set when the capture ended by itself.
    lost: Arc<AtomicBool>,
}

impl ThreadState {
    /// Counts a packet and queues it, dropping it when the queue is full.
    fn deliver(&self, packet: AudioPacket) {
        self.packets.fetch_add(1, Ordering::Relaxed);
        if let Err(TrySendError::Full(_)) = self.sender.try_send(packet) {
            self.overflows.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl AppLoopback {
    /// Starts capturing the app. `Include` hears every top-level process of the
    /// executable with its children, as they are running now. `Exclude` hears
    /// everything except the oldest top-level process and its children. Fails
    /// when the app is not running.
    pub fn start(target: AppTarget, mode: LoopbackMode) -> Result<Self, CaptureError> {
        let mut table = process_table()?;
        fill_creation_times(&mut table, target.exe());
        let mut pids: Vec<u32> = top_level_processes(&table, target.exe()).iter().map(|process| process.pid).collect();
        let total_instances = pids.len();
        if total_instances == 0 {
            return Err(CaptureError::Missing("no running process matches the app"));
        }
        let excluded_instances = match mode {
            LoopbackMode::Include => 0,
            LoopbackMode::Exclude => {
                // Windows can only leave out one process tree per stream.
                pids.truncate(1);
                pids.len()
            }
        };
        let event = create_event()?;
        let stop_flag = Arc::new(AtomicBool::new(false));
        let packets = Arc::new(AtomicU64::new(0));
        let overflows = Arc::new(AtomicU64::new(0));
        let lost = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let state = ThreadState {
            exe: target.exe().to_owned(),
            pids,
            mode,
            stop_flag: Arc::clone(&stop_flag),
            event,
            sender,
            ready_sender,
            packets: Arc::clone(&packets),
            overflows: Arc::clone(&overflows),
            lost: Arc::clone(&lost),
        };
        let spawned = thread::Builder::new().name(format!("audio-app-{}", state.exe)).spawn(move || capture_thread(&state));
        // From here on, dropping the handle joins the thread and closes the event.
        let mut capture = Self { stop_flag, event, thread: None, receiver, packets, overflows, lost, excluded_instances, total_instances };
        capture.thread = Some(spawned.map_err(|_| CaptureError::Missing("failed to spawn the app audio capture thread"))?);
        ready_receiver.recv().map_err(|_| CaptureError::Missing("the app audio capture thread exited before setup finished"))??;
        Ok(capture)
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

    /// True once every target process has exited, or the streams failed; the
    /// thread has ended and a new capture is needed.
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    /// How many instances of the app are left out of the sound: 1 in Exclude mode, else 0.
    pub fn excluded_instances(&self) -> usize {
        self.excluded_instances
    }

    /// How many instances (top-level processes) of the app ran when the capture
    /// started. In Exclude mode, more than `excluded_instances` means the others are heard.
    pub fn total_instances(&self) -> usize {
        self.total_instances
    }

    /// Stops the capture and joins its thread; Drop does the same.
    pub fn stop(self) {}
}

impl Drop for AppLoopback {
    #[allow(unsafe_code)]
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

/// One process loopback stream, created on the capture thread.
struct Stream {
    /// The initialized audio client, kept alive for the whole capture.
    client: IAudioClient,
    /// The capture side of the client.
    capture: IAudioCaptureClient,
    /// False once reading it failed.
    alive: bool,
}

/// A target process whose exit ends its part of the capture.
struct Target {
    pid: u32,
    /// None when Windows refuses a handle; the process table is read instead.
    handle: Option<ProcessHandle>,
    exited: bool,
}

/// Marks the targets that have exited; true when all of them have.
fn all_exited(targets: &mut [Target], table_checked: &mut Instant) -> bool {
    let read_table = targets.iter().any(|target| !target.exited && target.handle.is_none()) && table_checked.elapsed() >= TABLE_CHECK_INTERVAL;
    let table = if read_table {
        *table_checked = Instant::now();
        process_table().ok()
    } else {
        None
    };
    for target in targets.iter_mut().filter(|target| !target.exited) {
        target.exited = match (&target.handle, &table) {
            (Some(handle), _) => handle.has_exited(),
            (None, Some(table)) => !table.iter().any(|process| process.pid == target.pid),
            (None, None) => false,
        };
    }
    targets.iter().all(|target| target.exited)
}

/// Runs on the capture thread: COM init, setup, loop, and clean COM shutdown.
fn capture_thread(state: &ThreadState) {
    let owns_com = init_com();
    // The handles are opened before the streams so a reused process ID cannot slip in later.
    let targets: Vec<Target> = state.pids.iter().map(|&pid| Target { pid, handle: ProcessHandle::open_for_exit(pid), exited: false }).collect();
    match state.pids.iter().map(|&pid| open_stream(pid, state.mode, state.event)).collect::<Result<Vec<Stream>, CaptureError>>() {
        Ok(mut streams) => {
            let _ = state.ready_sender.send(Ok(()));
            capture_loop(&mut streams, targets, state);
            for stream in streams.iter().filter(|stream| stream.alive) {
                if let Err(err) = stop_client(&stream.client) {
                    log::warn!("app audio capture of {}: {err}", state.exe);
                }
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

/// Pumps packets until stopped, every target has exited or every stream failed.
fn capture_loop(streams: &mut [Stream], mut targets: Vec<Target>, state: &ThreadState) {
    // One stream passes its packets through with the WASAPI timestamps untouched.
    let mut mixer = (streams.len() > 1).then(|| Mixer::new(streams.len()));
    let mut table_checked = Instant::now();
    loop {
        if wait_for_streams(state.event) == WAIT_FAILED {
            log::warn!("app audio capture of {}: the event wait failed", state.exe);
            break;
        }
        if state.stop_flag.load(Ordering::Relaxed) {
            return;
        }
        // Checked before draining, so the last packets of an exited app are still read.
        let ended = all_exited(&mut targets, &mut table_checked);
        for (index, stream) in streams.iter_mut().enumerate().filter(|(_, stream)| stream.alive) {
            if let Err(err) = drain_stream(&stream.capture, index, mixer.as_mut(), state) {
                log::warn!("app audio capture of {}: a stream failed: {err}", state.exe);
                stream.alive = false;
            }
        }
        if let Some(mixer) = mixer.as_mut() {
            while let Some(packet) = mixer.pop(qpc_now_100ns()) {
                state.deliver(packet);
            }
        }
        if ended || streams.iter().all(|stream| !stream.alive) {
            break;
        }
    }
    if let Some(mixer) = mixer.as_mut() {
        while let Some(packet) = mixer.flush() {
            state.deliver(packet);
        }
    }
    state.lost.store(true, Ordering::Relaxed);
}

/// Reads every waiting packet of one stream into the mixer, or straight to the queue.
fn drain_stream(capture: &IAudioCaptureClient, index: usize, mut mixer: Option<&mut Mixer>, state: &ThreadState) -> Result<(), CaptureError> {
    while with_context("failed to query the WASAPI packet size", next_packet_size(capture))? > 0 {
        let mut packet = match read_buffer(capture, APP_CHANNELS, APP_SAMPLE_RATE) {
            Ok(packet) => packet,
            Err(BufferError::Empty) => break,
            Err(BufferError::DeviceLost) => return Err(CaptureError::Missing("the process loopback stream was invalidated")),
            Err(BufferError::Failed(err)) => return Err(err),
        };
        packet.qpc_100ns = checked_stamp(packet.qpc_100ns, packet.frames, qpc_now_100ns());
        match mixer.as_deref_mut() {
            Some(mixer) => mixer.push(index, &packet),
            None => state.deliver(packet),
        }
    }
    Ok(())
}

/// The packet's WASAPI timestamp, or, when that is nowhere near now (process
/// loopback is not documented to fill it in), the time its first frame would
/// have if the packet had just ended.
fn checked_stamp(qpc_100ns: i64, frames: u32, now_100ns: i64) -> i64 {
    if qpc_100ns.abs_diff(now_100ns) <= MAX_STAMP_ERROR_100NS as u64 {
        qpc_100ns
    } else {
        now_100ns - i64::from(frames) * 10_000_000 / i64::from(APP_SAMPLE_RATE)
    }
}

/// Waits for any stream to signal, for at most one poll interval.
#[allow(unsafe_code)]
fn wait_for_streams(event: Event) -> windows::Win32::Foundation::WAIT_EVENT {
    // SAFETY: event is our live handle until the capture thread is joined.
    unsafe { WaitForSingleObject(event.handle(), POLL_MS) }
}

/// Activates, initializes and starts the loopback stream of one process tree.
fn open_stream(pid: u32, mode: LoopbackMode, event: Event) -> Result<Stream, CaptureError> {
    let client = activate_process_loopback(pid, mode)?;
    with_context("failed to initialize the process loopback client", initialize_loopback(&client))?;
    let capture = with_context("failed to get the WASAPI capture client", get_capture_client(&client))?;
    with_context("failed to set the capture event handle", set_event_handle(&client, event))?;
    with_context("failed to start the app audio capture", start_client(&client))?;
    Ok(Stream { client, capture, alive: true })
}

/// Initializes a process loopback client for 48 kHz stereo float. Such a
/// client has no mix format to ask for.
#[allow(unsafe_code)]
fn initialize_loopback(client: &IAudioClient) -> windows::core::Result<()> {
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: APP_CHANNELS,
        nSamplesPerSec: APP_SAMPLE_RATE,
        nAvgBytesPerSec: APP_SAMPLE_RATE * u32::from(APP_CHANNELS) * 4,
        nBlockAlign: APP_CHANNELS * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
    // SAFETY: format is a valid WAVEFORMATEX for this call, and no session GUID is passed.
    unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_100NS, 0, &format, None) }
}

/// Asks Windows for an audio client that captures one process tree, or all but it.
#[allow(unsafe_code)]
fn activate_process_loopback(pid: u32, mode: LoopbackMode) -> Result<IAudioClient, CaptureError> {
    // On the heap so that an activation abandoned after a timeout can be left its parameters.
    let mut params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: match mode {
                    LoopbackMode::Include => PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                    LoopbackMode::Exclude => PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
                },
            },
        },
    });
    let blob = BLOB { cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: (&raw mut *params).cast() };
    // The blob borrows `params`, so the PROPVARIANT must never be cleared: its Drop would free the pointer.
    let variant = Box::new(ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 { vt: VT_BLOB, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: PROPVARIANT_0_0_0 { blob } }),
        },
    }));
    let (done_sender, done) = mpsc::channel();
    let handler = completion_handler(done_sender);
    // SAFETY: the variant and the params it points at live until the activation has finished, or forever after a timeout.
    let started = unsafe { ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&**variant), &handler) };
    let operation: IActivateAudioInterfaceAsyncOperation =
        with_context("failed to request process loopback (this version of Windows may not support it)", started)?;
    if done.recv_timeout(ACTIVATION_TIMEOUT).is_err() {
        Box::leak(variant);
        Box::leak(params);
        return Err(CaptureError::Missing("process loopback activation timed out"));
    }
    let mut result = S_OK;
    let mut activated: Option<IUnknown> = None;
    // SAFETY: the activation completed; both out pointers point at locals.
    with_context("failed to read the activation result", unsafe { operation.GetActivateResult(&mut result, &mut activated) })?;
    with_context("process loopback activation failed", result.ok())?;
    let activated = activated.ok_or(CaptureError::Missing("process loopback activation returned no interface"))?;
    with_context("the activated interface is not an audio client", activated.cast())
}

// The completion handler is written out by hand: the `implement` macro would
// make windows-core a direct dependency. It is a reference counted COM object
// that is agile, as ActivateAudioInterfaceAsync requires.

/// The COM object behind the completion handler; the vtable pointer comes first.
#[repr(C)]
struct Completion {
    vtable: *const IActivateAudioInterfaceCompletionHandler_Vtbl,
    references: AtomicU32,
    done: mpsc::Sender<()>,
}

/// The methods of every completion handler.
static COMPLETION_VTABLE: IActivateAudioInterfaceCompletionHandler_Vtbl = IActivateAudioInterfaceCompletionHandler_Vtbl {
    base__: IUnknown_Vtbl { QueryInterface: completion_query_interface, AddRef: completion_add_ref, Release: completion_release },
    ActivateCompleted: completion_activate_completed,
};

/// A completion handler that sends on `done` when the activation has finished.
#[allow(unsafe_code)]
fn completion_handler(done: mpsc::Sender<()>) -> IActivateAudioInterfaceCompletionHandler {
    let object = Box::new(Completion { vtable: &COMPLETION_VTABLE, references: AtomicU32::new(1), done });
    // SAFETY: the pointer is a COM object with the interface's vtable; the interface takes its one reference.
    unsafe { IActivateAudioInterfaceCompletionHandler::from_raw(Box::into_raw(object).cast()) }
}

/// IUnknown::QueryInterface: the object is its own IUnknown, handler and agile marker.
#[allow(unsafe_code)]
unsafe extern "system" fn completion_query_interface(this: *mut c_void, iid: *const GUID, interface: *mut *mut c_void) -> HRESULT {
    if iid.is_null() || interface.is_null() {
        return E_POINTER;
    }
    // SAFETY: COM passes a valid IID.
    let iid = unsafe { *iid };
    if iid == IUnknown::IID || iid == IActivateAudioInterfaceCompletionHandler::IID || iid == IID_AGILE_OBJECT {
        // SAFETY: this is a live Completion, and interface is a valid out pointer.
        unsafe {
            completion_add_ref(this);
            *interface = this;
        }
        S_OK
    } else {
        // SAFETY: interface is a valid out pointer.
        unsafe { *interface = std::ptr::null_mut() };
        E_NOINTERFACE
    }
}

/// IUnknown::AddRef.
#[allow(unsafe_code)]
unsafe extern "system" fn completion_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: COM only calls this on a live Completion.
    let object = unsafe { &*this.cast::<Completion>() };
    object.references.fetch_add(1, Ordering::Relaxed) + 1
}

/// IUnknown::Release; the last one frees the object.
#[allow(unsafe_code)]
unsafe extern "system" fn completion_release(this: *mut c_void) -> u32 {
    // SAFETY: COM only calls this on a live Completion.
    let object = unsafe { &*this.cast::<Completion>() };
    let left = object.references.fetch_sub(1, Ordering::Release) - 1;
    if left == 0 {
        fence(Ordering::Acquire);
        // SAFETY: that was the last reference to the box made in completion_handler.
        drop(unsafe { Box::from_raw(this.cast::<Completion>()) });
    }
    left
}

/// IActivateAudioInterfaceCompletionHandler::ActivateCompleted, on a Windows worker thread.
#[allow(unsafe_code)]
unsafe extern "system" fn completion_activate_completed(this: *mut c_void, _operation: *mut c_void) -> HRESULT {
    // SAFETY: COM holds a reference while it calls.
    let object = unsafe { &*this.cast::<Completion>() };
    // The receiver is gone only after a timeout.
    let _ = object.done.send(());
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_far_from_now_is_replaced_by_the_packets_own_age() {
        let now = 900_000_000_000;
        assert_eq!(checked_stamp(now - 200_000, 480, now), now - 200_000, "a believable stamp is kept");
        assert_eq!(checked_stamp(0, 480, now), now - 100_000, "480 frames are 10 ms");
    }

    /// The hand written COM object answers for its interfaces and counts references.
    #[test]
    fn the_completion_handler_is_an_agile_reference_counted_com_object() -> windows::core::Result<()> {
        let (sender, done) = mpsc::channel();
        let handler = completion_handler(sender);
        let unknown: IUnknown = handler.cast()?;
        assert_eq!(unknown.as_raw(), handler.as_raw());
        drop(unknown);
        drop(handler);
        assert!(done.try_recv() == Err(mpsc::TryRecvError::Disconnected), "the last release freed the object and its sender");
        Ok(())
    }
}
