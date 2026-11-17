//! Probes the AMF encoder path: prints the AMF encoders' private options, then
//! encodes three seconds of the primary monitor to probe.ivf as AV1.

use std::ffi::{CStr, CString, c_void};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use capture::{Gpu, LatestFrame, MonitorCapture, Nv12Converter, list_monitors};
use ffmpeg_next::ffi;
use media_io::encode::{Codec, Encoder, Preset, RateControl};
use media_io::{AmfEncoder, D3d11Device, D3d11FramePool, EncodedPacket, EncoderConfig, HwFrame};

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

    let mut writer = IvfWriter::create(Path::new("probe.ivf"), 1920, 1080, 60, 1)?;
    let mut stats = Stats::default();
    let mut frames_sent = 0u64;
    let mut encode_ns = 0u128;

    let period = Duration::from_nanos(1_000_000_000 / 60);
    let start = Instant::now();
    let mut next_tick = start;
    while start.elapsed() < Duration::from_secs(3) {
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
        let begin = Instant::now();
        encoder.send(hw)?;
        encoder.receive(&mut |packet| record_packet(&packet, &mut writer, &mut stats))?;
        encode_ns += begin.elapsed().as_nanos();
        frames_sent += 1;
    }
    encoder.finish(&mut |packet| record_packet(&packet, &mut writer, &mut stats))?;
    if let Some(err) = writer.take_error() {
        return Err(err.into());
    }
    writer.finish()?;
    capture.stop();

    let seconds = start.elapsed().as_secs_f64();
    let mbps = stats.bytes as f64 * 8.0 / seconds / 1_000_000.0;
    let ms_per_frame = if frames_sent > 0 { encode_ns as f64 / 1_000_000.0 / frames_sent as f64 } else { 0.0 };
    println!(
        "frames sent: {frames_sent}, packets: {}, keyframes: {}, bytes: {}, average {:.2} Mbps, average send+receive {:.2} ms/frame",
        stats.packets, stats.keyframes, stats.bytes, mbps, ms_per_frame
    );
    println!("wrote probe.ivf");
    Ok(())
}

/// Waits for the first captured frame.
fn wait_for_frame(capture: &MonitorCapture) -> Option<LatestFrame> {
    for _ in 0..100 {
        if let Some(frame) = capture.latest() {
            return Some(frame);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// Running totals for the probe.
#[derive(Default)]
struct Stats {
    packets: u64,
    keyframes: u64,
    bytes: u64,
}

/// Records one packet into the writer and the stats.
fn record_packet(packet: &EncodedPacket, writer: &mut IvfWriter, stats: &mut Stats) {
    stats.packets += 1;
    if packet.is_key() {
        stats.keyframes += 1;
    }
    stats.bytes += packet.size() as u64;
    let pts = packet.pts().unwrap_or(0).max(0) as u64;
    writer.write_packet(pts, packet.data());
}

/// A minimal IVF writer for the AV1 probe output.
struct IvfWriter {
    file: File,
    frames: u32,
    error: Option<std::io::Error>,
}

impl IvfWriter {
    /// Creates the file and writes the 32-byte IVF header.
    fn create(path: &Path, width: u16, height: u16, timebase_den: u32, timebase_num: u32) -> std::io::Result<Self> {
        let mut file = File::create(path)?;
        let mut header = [0u8; 32];
        header[0..4].copy_from_slice(b"DKIF");
        header[4..6].copy_from_slice(&0u16.to_le_bytes());
        header[6..8].copy_from_slice(&32u16.to_le_bytes());
        header[8..12].copy_from_slice(b"AV01");
        header[12..14].copy_from_slice(&width.to_le_bytes());
        header[14..16].copy_from_slice(&height.to_le_bytes());
        header[16..20].copy_from_slice(&timebase_den.to_le_bytes());
        header[20..24].copy_from_slice(&timebase_num.to_le_bytes());
        file.write_all(&header)?;
        Ok(Self { file, frames: 0, error: None })
    }

    /// Writes the 12-byte frame header and the payload, keeping the first error.
    fn write_packet(&mut self, pts: u64, data: Option<&[u8]>) {
        if self.error.is_some() {
            return;
        }
        let data = data.unwrap_or(&[]);
        let result = (|| -> std::io::Result<()> {
            self.file.write_all(&(data.len() as u32).to_le_bytes())?;
            self.file.write_all(&pts.to_le_bytes())?;
            self.file.write_all(data)?;
            Ok(())
        })();
        if let Err(err) = result {
            self.error = Some(err);
            return;
        }
        self.frames += 1;
    }

    /// Rewrites the frame count in the header and flushes.
    fn finish(&mut self) -> std::io::Result<()> {
        self.file.seek(SeekFrom::Start(24))?;
        self.file.write_all(&self.frames.to_le_bytes())?;
        self.file.flush()
    }

    /// The first write error, if any.
    fn take_error(&mut self) -> Option<std::io::Error> {
        self.error.take()
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
