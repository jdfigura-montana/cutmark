//! Probes the AMF encoder path. With --probe it only prints which AMF codecs open and the
//! option ranges they report. Otherwise it prints the AMF encoders' private options, then
//! encodes the primary monitor to probe_<codec>_<rc>.mkv.
//!
//! Usage: encode_probe [--probe] [--codec av1|hevc|h264] [--rc cq|cqp|cbr|vbr] [--quality N] [--seconds N] [--source screen|noise]
//! The cq output is named probe_<codec>_cq<N>.mkv; --quality (1 to 100) only applies to --rc cq.
//! The default source is the primary monitor. The noise source fills every frame with fresh
//! seeded pseudo-random 16x16 blocks, which compress badly, and takes the same upload and
//! NV12 path. Before opening, the option list the configuration maps to is printed.

use std::ffi::{CStr, CString, c_void};
use std::path::Path;
use std::time::{Duration, Instant};

use capture::{Gpu, LatestFrame, MonitorCapture, Nv12Converter, list_monitors};
use ffmpeg_next::ffi;
use media_io::encode::{Codec, CodecSupport, Encoder, Preset, RateControl, amf_options, probe_amf};
use media_io::{AmfEncoder, Container, D3d11Device, D3d11FramePool, EncodedPacket, EncoderConfig, HwFrame, Muxer};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_WRITE, D3D11_MAP_WRITE, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

/// The noise source's frame size.
const NOISE_WIDTH: usize = 1920;
/// The noise source's frame height.
const NOISE_HEIGHT: usize = 1080;
/// The side of one noise block, in pixels.
const NOISE_BLOCK: usize = 16;
/// The fixed seed that makes the noise sequence repeatable.
const NOISE_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// The command line options.
struct Args {
    probe_only: bool,
    noise: bool,
    codec: Codec,
    codec_name: &'static str,
    rc_name: String,
    rate_control: RateControl,
    seconds: u64,
}

/// Parses the command line, with the documented defaults.
fn parse_args() -> Result<Args, String> {
    let mut probe_only = false;
    let mut noise = false;
    let mut codec = (Codec::Av1, "av1");
    let mut rc = ("cq", RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 });
    let mut seconds = 10u64;
    let mut quality: Option<u8> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--probe" => probe_only = true,
            "--codec" => {
                codec = match args.next().as_deref() {
                    Some("av1") => (Codec::Av1, "av1"),
                    Some("hevc") => (Codec::Hevc, "hevc"),
                    Some("h264") => (Codec::H264, "h264"),
                    other => return Err(format!("--codec wants av1, hevc or h264, got {other:?}")),
                };
            }
            "--rc" => {
                rc = match args.next().as_deref() {
                    Some("cq") => ("cq", RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 }),
                    Some("cqp") => ("cqp", RateControl::ConstantQp { qp_i: 24, qp_p: 26 }),
                    Some("cbr") => ("cbr", RateControl::Cbr { mbps: 20.0 }),
                    Some("vbr") => ("vbr", RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 }),
                    other => return Err(format!("--rc wants cq, cqp, cbr or vbr, got {other:?}")),
                };
            }
            "--quality" => {
                quality = Some(
                    args.next()
                        .and_then(|value| value.parse::<u8>().ok())
                        .filter(|n| (1..=100).contains(n))
                        .ok_or("--quality wants a whole number from 1 to 100")?,
                );
            }
            "--source" => {
                noise = match args.next().as_deref() {
                    Some("screen") => false,
                    Some("noise") => true,
                    other => return Err(format!("--source wants screen or noise, got {other:?}")),
                };
            }
            "--seconds" => {
                seconds = args.next().and_then(|value| value.parse().ok()).filter(|&n| n > 0).ok_or("--seconds wants a whole number above 0")?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let mut rc_name = rc.0.to_string();
    if let Some(quality) = quality {
        match &mut rc.1 {
            RateControl::ConstantQuality { quality: current, .. } => {
                *current = quality;
                rc_name = format!("cq{quality}");
            }
            _ => return Err("--quality only applies to --rc cq".to_string()),
        }
    } else if let RateControl::ConstantQuality { quality, .. } = rc.1 {
        rc_name = format!("cq{quality}");
    }
    Ok(Args { probe_only, noise, codec: codec.0, codec_name: codec.1, rc_name, rate_control: rc.1, seconds })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    media_io::init()?;
    if args.probe_only {
        print_probe_table(&probe_amf(1920, 1080, 60));
        return Ok(());
    }
    print_amf_options("av1_amf");
    print_amf_options("hevc_amf");
    print_amf_options("h264_amf");

    let gpu = Gpu::create(Some(0))?;
    let hw_device = D3d11Device::from_device(gpu.device())?;
    let (capture, mut noise, source_size) = if args.noise {
        let noise = NoiseSource::create(&gpu)?;
        println!("source: noise {NOISE_WIDTH}x{NOISE_HEIGHT}");
        (None, Some(noise), (NOISE_WIDTH as u32, NOISE_HEIGHT as u32))
    } else {
        let monitor = list_monitors()?.into_iter().find(|m| m.primary).ok_or("no primary monitor")?;
        let capture = MonitorCapture::start(&gpu, &monitor, false)?;
        let first = wait_for_frame(&capture).ok_or("no frame arrived from the capture session")?;
        println!("captured {}x{}", first.width, first.height);
        (Some(capture), None, (first.width as u32, first.height as u32))
    };

    let mut converter = Nv12Converter::new(&gpu, source_size.0, source_size.1, 1920, 1080, 60)?;
    let nv12 = converter.create_output()?;
    let pool = D3d11FramePool::new(&hw_device, 1920, 1080, 8)?;
    println!("pool mode: {:?}", pool.mode());

    let cfg = EncoderConfig {
        codec: args.codec,
        width: 1920,
        height: 1080,
        fps_num: 60,
        fps_den: 1,
        rate_control: args.rate_control,
        preset: Preset::Balanced,
        keyframe_secs: 2.0,
    };
    println!("config: {cfg:?}");
    print_mapped_options(&cfg);
    let mut encoder = AmfEncoder::open(&cfg, &pool)?;
    println!("encoder time base: {:?}", encoder.time_base());

    let path_text = format!("probe_{}_{}.mkv", args.codec_name, args.rc_name);
    let mut muxer = Muxer::create(Path::new(&path_text), Container::Mkv)?;
    let stream = muxer.add_video(encoder.parameters(), encoder.time_base(), (60, 1))?;
    muxer.start()?;
    let mut sink = Sink { muxer, stream, stats: Stats::default(), error: None };
    let mut frames_sent = 0u64;
    let mut encode_ns = 0u128;

    let period = Duration::from_nanos(1_000_000_000 / 60);
    let start = Instant::now();
    let mut next_tick = start;
    while start.elapsed() < Duration::from_secs(args.seconds) {
        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
        next_tick += period;
        if let Some(noise) = noise.as_mut() {
            noise.upload_next(gpu.context())?;
            converter.convert(noise.texture(), &nv12)?;
        } else {
            let Some(frame) = capture.as_ref().and_then(|capture| capture.latest()) else {
                continue;
            };
            converter.convert(&frame.texture, &nv12)?;
        }
        let mut hw = HwFrame::from_pool(&pool)?;
        hw.copy_from(gpu.context(), &nv12)?;
        hw.set_pts(frames_sent as i64);
        let begin = Instant::now();
        encoder.send(hw)?;
        encoder.receive(&mut |packet| sink.record(&packet))?;
        encode_ns += begin.elapsed().as_nanos();
        frames_sent += 1;
    }
    encoder.finish(&mut |packet| sink.record(&packet))?;
    if let Some(capture) = capture {
        capture.stop();
    }
    if let Some(err) = sink.error {
        return Err(err.into());
    }
    let stats = sink.stats;
    sink.muxer.finish()?;

    let seconds = start.elapsed().as_secs_f64();
    let mbps = stats.bytes as f64 * 8.0 / seconds / 1_000_000.0;
    let ms_per_frame = if frames_sent > 0 { encode_ns as f64 / 1_000_000.0 / frames_sent as f64 } else { 0.0 };
    println!(
        "frames sent: {frames_sent}, packets: {}, keyframes: {}, bytes: {}, average {:.2} Mbps, average send+receive {:.2} ms/frame",
        stats.packets, stats.keyframes, stats.bytes, mbps, ms_per_frame
    );
    println!("wrote {path_text}");
    Ok(())
}

/// Prints the option list the configuration maps to on its codec, one line per option.
fn print_mapped_options(cfg: &EncoderConfig) {
    let support = probe_amf(cfg.width, cfg.height, cfg.fps_num);
    let Some(row) = support.iter().find(|row| row.codec == cfg.codec) else {
        println!("amf options: codec not probed");
        return;
    };
    match amf_options(cfg.codec, cfg, &row.ranges) {
        Ok(options) => {
            println!("amf options for {:?}:", cfg.codec);
            for (name, value) in options {
                println!("    {name} = {value}");
            }
        }
        Err(err) => println!("amf options: {err}"),
    }
}

/// A texture refilled every frame with fresh seeded pseudo-random blocks. The noise is
/// written into a CPU-writable staging texture, then copied into a default-usage
/// texture, which is what the video processor accepts as input.
struct NoiseSource {
    staging: ID3D11Texture2D,
    texture: ID3D11Texture2D,
    pixels: Vec<u8>,
    state: u64,
}

impl NoiseSource {
    /// Creates the staging texture and the default-usage BGRA texture the converter reads from.
    #[allow(unsafe_code)]
    fn create(gpu: &Gpu) -> Result<Self, Box<dyn std::error::Error>> {
        let mut desc = D3D11_TEXTURE2D_DESC {
            Width: NOISE_WIDTH as u32,
            Height: NOISE_HEIGHT as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        // SAFETY: the descriptor and the out pointer are live locals for the call.
        unsafe { gpu.device().CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.ok_or("CreateTexture2D returned no texture")?;

        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE.0 as u32;
        let mut staging: Option<ID3D11Texture2D> = None;
        // SAFETY: the descriptor and the out pointer are live locals for the call.
        unsafe { gpu.device().CreateTexture2D(&desc, None, Some(&mut staging))? };
        let staging = staging.ok_or("CreateTexture2D returned no staging texture")?;
        Ok(Self { staging, texture, pixels: vec![0; NOISE_WIDTH * NOISE_HEIGHT * 4], state: NOISE_SEED })
    }

    /// The default-usage texture holding the latest noise frame.
    fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }

    /// Fills a fresh frame of noise, writes it into the staging texture and copies it across.
    #[allow(unsafe_code)]
    fn upload_next(&mut self, context: &ID3D11DeviceContext) -> Result<(), Box<dyn std::error::Error>> {
        fill_noise(&mut self.pixels, NOISE_WIDTH, NOISE_HEIGHT, &mut self.state);
        let row_bytes = NOISE_WIDTH * 4;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture was created with CPU write access and the out pointer
        // is a live local.
        unsafe { context.Map(&self.staging, 0, D3D11_MAP_WRITE, 0, Some(&mut mapped))? };
        for (row, source) in self.pixels.chunks_exact(row_bytes).enumerate() {
            // SAFETY: the mapping spans NOISE_HEIGHT rows of RowPitch bytes, each at least
            // row_bytes long, and the source row is row_bytes long.
            unsafe {
                let destination = mapped.pData.cast::<u8>().add(row * mapped.RowPitch as usize);
                std::ptr::copy_nonoverlapping(source.as_ptr(), destination, row_bytes);
            }
        }
        // SAFETY: the staging texture is mapped by the call above.
        unsafe {
            context.Unmap(&self.staging, 0);
            context.CopyResource(&self.texture, &self.staging);
        }
        Ok(())
    }
}

/// The next value of a xorshift64* sequence.
fn next_random(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Fills a BGRA buffer with random pixels, one 16x16 block at a time, fully opaque.
fn fill_noise(pixels: &mut [u8], width: usize, height: usize, state: &mut u64) {
    for block_y in (0..height).step_by(NOISE_BLOCK) {
        for block_x in (0..width).step_by(NOISE_BLOCK) {
            let rows = NOISE_BLOCK.min(height - block_y);
            let columns = NOISE_BLOCK.min(width - block_x);
            for row in 0..rows {
                let start = ((block_y + row) * width + block_x) * 4;
                for pixel in pixels[start..start + columns * 4].as_chunks_mut::<4>().0 {
                    let random = next_random(state).to_le_bytes();
                    *pixel = [random[0], random[1], random[2], 255];
                }
            }
        }
    }
}

/// Prints one row per probed codec: status, then the option ranges the encoder reported.
fn print_probe_table(support: &[CodecSupport]) {
    println!("{:<6} {:<8} {:<26} {:<16} {:<16} rc modes", "codec", "status", "quality level", "qp_i", "qp_p");
    for row in support {
        let status = if row.result.is_ok() { "ok" } else { "failed" };
        println!(
            "{:<6} {:<8} {:<26} {:<16} {:<16} {}",
            format!("{:?}", row.codec),
            status,
            describe_range(row.ranges.quality_level.as_ref()),
            describe_range(row.ranges.qp_i.as_ref()),
            describe_range(row.ranges.qp_p.as_ref()),
            row.ranges.rc_modes.join(",")
        );
        if let Err(reason) = &row.result {
            println!("       error: {reason}");
        }
    }
}

/// Formats one numeric option as "name min..max", or "none".
fn describe_range(option: Option<&media_io::encode::NumericOption>) -> String {
    match option {
        Some(option) => format!("{} {}..{}", option.name, option.min, option.max),
        None => "none".to_string(),
    }
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

/// Where packets go: the muxer, the totals, and the first write error.
struct Sink {
    muxer: Muxer,
    stream: usize,
    stats: Stats,
    error: Option<media_io::MediaError>,
}

impl Sink {
    /// Records one packet into the totals and the muxer, keeping the first error.
    fn record(&mut self, packet: &EncodedPacket) {
        self.stats.packets += 1;
        if packet.is_key() {
            self.stats.keyframes += 1;
        }
        self.stats.bytes += packet.size() as u64;
        if self.error.is_some() {
            return;
        }
        if let Err(err) = self.muxer.write(self.stream, packet) {
            self.error = Some(err);
        }
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
