//! Probes the encode plus mux path: encodes ten seconds of the primary
//! monitor to probe.mkv through the Muxer, then prints the totals.

use std::ffi::{CStr, CString, c_void};
use std::path::Path;
use std::time::{Duration, Instant};

use capture::{Gpu, MonitorCapture, Nv12Converter, list_monitors};
use ffmpeg_next::ffi;
use media_io::encode::{Codec, Encoder, Preset, RateControl};
use media_io::mux::{Container, Muxer};
use media_io::{AmfEncoder, D3d11Device, D3d11FramePool, EncoderConfig, HwFrame};

/// Running totals collected in the packet callback.
#[derive(Default)]
struct Stats {
    packets: u64,
    keyframes: u64,
    bytes: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    media_io::init()?;
    print_amf_options("av1_amf");
    print_amf_options("hevc_amf");
    print_amf_options("h264_amf");

    let gpu = Gpu::create(Some(0))?;
    let hw_device = D3d11Device::from_device(gpu.device())?;
    let monitor = list_monitors()?.into_iter().find(|m| m.primary).ok_or("no primary monitor")?;
    let capture = MonitorCapture::start(&gpu, &monitor, false)?;
    let first = wait_for_frame(&capture).ok_or("no frame arrived from the capture session")?;
    println!("captured {}x{}", first.width, first.height);

    let mut converter = Nv12Converter::new(&gpu, first.width as u32, first.height as u32, 1920, 1080, 60)?;
    let nv12 = converter.create_output()?;
    let pool = D3d11FramePool::new(&hw_device, 1920, 1080, 8)?;
    println!("pool mode: {:?}", pool.mode());

    let cfg = EncoderConfig {
        codec: Codec::Av1,
        width: 1920,
        height: 1080,
        fps_num: 60,
        fps_den: 1,
        rate_control: RateControl::Qvbr { quality: 23, max_mbps: 20 },
        preset: Preset::Balanced,
        keyframe_seconds: 2,
    };
    let mut encoder = AmfEncoder::open(&cfg, &pool)?;
    println!("encoder time base: {:?}", encoder.time_base());

    let mut mux = Muxer::create(Path::new("probe.mkv"), Container::Mkv)?;
    let stream = mux.add_video(encoder.parameters(), encoder.time_base(), (60, 1))?;
    mux.start()?;

    let mut stats = Stats::default();
    let mut frames_sent = 0u64;

    let period = Duration::from_nanos(1_000_000_000 / 60);
    let start = Instant::now();
    let mut next_tick = start;
    while start.elapsed() < Duration::from_secs(10) {
        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
        next_tick += period;
        let Some(frame) = capture.latest() else {
            continue;
        };
        converter.convert(&frame.texture, &nv12)?;
        let mut hw = HwFrame::from_pool(&pool)?;
        hw.copy_from(gpu.context(), &nv12)?;
        hw.set_pts(frames_sent as i64);
        encoder.send(hw)?;
        encoder.receive(&mut |packet| record_packet(&packet, &mut mux, stream, &mut stats))?;
        frames_sent += 1;
    }
    encoder.finish(&mut |packet| record_packet(&packet, &mut mux, stream, &mut stats))?;
    mux.finish()?;
    capture.stop();

    let seconds = start.elapsed().as_secs_f64();
    let mbps = stats.bytes as f64 * 8.0 / seconds / 1_000_000.0;
    println!("frames sent: {frames_sent}, packets: {}, keyframes: {}, bytes: {}, average {:.2} Mbps", stats.packets, stats.keyframes, stats.bytes, mbps);
    Ok(())
}

/// Waits up to one second for the first capture frame.
fn wait_for_frame(capture: &MonitorCapture) -> Option<capture::LatestFrame> {
    for _ in 0..100 {
        if let Some(frame) = capture.latest() {
            return Some(frame);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// Writes one packet into the MKV and adds it to the running totals.
fn record_packet(packet: &media_io::EncodedPacket, mux: &mut Muxer, stream: usize, stats: &mut Stats) {
    stats.packets += 1;
    if packet.is_key() {
        stats.keyframes += 1;
    }
    stats.bytes += packet.size() as u64;
    if let Err(err) = mux.write(stream, packet) {
        eprintln!("mux write failed: {err}");
    }
}

/// Prints the private option table of one encoder, with its named values.
#[allow(unsafe_code)]
fn print_amf_options(name: &str) {
    let cname = match CString::new(name) {
        Ok(cname) => cname,
        Err(_) => return,
    };
    // SAFETY: cname outlives the call; the codec table is static.
    let codec = unsafe { ffi::avcodec_find_encoder_by_name(cname.as_ptr()) };
    if codec.is_null() {
        println!("{name}: not in this FFmpeg build");
        return;
    }
    // SAFETY: codec is a live static AVCodec.
    let class = unsafe { (*codec).priv_class };
    if class.is_null() {
        println!("{name}: no private options");
        return;
    }
    println!("{name} options:");
    let mut prev: *const ffi::AVOption = std::ptr::null();
    loop {
        // SAFETY: the fake object is a pointer to an AVClass pointer, which is
        // what av_opt_next reads; the table is static.
        let entry = unsafe { ffi::av_opt_next(std::ptr::from_ref(&class).cast::<c_void>(), prev) };
        if entry.is_null() {
            break;
        }
        prev = entry;
        // SAFETY: entry points into the static option table; every field read
        // is a static string or a plain integer.
        let text = unsafe {
            let option = CStr::from_ptr((*entry).name).to_string_lossy().into_owned();
            let option_type = (*entry).type_;
            let unit = if (*entry).unit.is_null() { String::new() } else { CStr::from_ptr((*entry).unit).to_string_lossy().into_owned() };
            (option, option_type, unit)
        };
        if text.1 == ffi::AVOptionType::AV_OPT_TYPE_CONST {
            println!("    {} (named value under {})", text.0, text.2);
        } else {
            let kind = ffmpeg_next::util::option::Type::from(text.1);
            println!("    {} ({:?})", text.0, kind);
        }
    }
}
