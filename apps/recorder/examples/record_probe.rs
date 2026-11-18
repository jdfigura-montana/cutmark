//! Records the primary monitor through the engine session with two audio
//! tracks, WAV sidecars, a mixdown and per-track streams, then prints the
//! totals. Usage: `cargo run -p cutmark-recorder --example record_probe
//! [seconds] [output.mkv]`

use std::path::PathBuf;
use std::time::Duration;

// The Recorder app has no lib target, so the example compiles the modules it
// needs straight from src; it uses only a subset of each.
#[allow(dead_code)]
#[path = "../src/engine/audio.rs"]
mod audio;
#[allow(dead_code)]
#[path = "../src/engine/audio_track.rs"]
mod audio_track;
#[allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[allow(dead_code)]
#[path = "../src/engine/pacer.rs"]
mod pacer;
#[allow(dead_code)]
#[path = "../src/engine/session.rs"]
mod session;
#[allow(dead_code)]
#[path = "../src/engine/video.rs"]
mod video;

use capture::AudioDevice;
use config::Profile;
use media_io::AudioCodec;
use session::{RecordingSession, SessionConfig, TrackSpec};

/// Builds the tracks from the suite default rule in config::default_tracks,
/// each bound to its device from the list.
fn default_track_specs(devices: &[AudioDevice]) -> Vec<TrackSpec> {
    config::default_tracks(devices)
        .into_iter()
        .map(|track| TrackSpec {
            device: devices.iter().find(|device| device.id == track.device_id).cloned(),
            name: track.name,
            device_id: track.device_id,
            device_name: track.device_name,
            gain_db: track.gain_db,
            muted: track.muted,
            offset_ms: track.offset_ms,
            in_mixdown: track.in_mixdown,
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let seconds: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(10);
    let output = args.next().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("record_probe.mkv"));

    let devices = capture::list_audio_devices()?;
    let tracks = default_track_specs(&devices);
    let take_folder = output.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let base_name = output.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "take".to_string());
    let cfg = SessionConfig {
        adapter: 0,
        monitor: 0,
        cursor: true,
        profile: Profile::tutorial_av1_1440p60(),
        output_file: output.clone(),
        tracks,
        audio_codec: AudioCodec::Aac { kbps: 320 },
        mixdown: true,
        track_streams: true,
        wav_sidecars: true,
        take_folder,
        base_name,
    };
    let session = RecordingSession::start(cfg)?;
    println!("recording for {seconds} s ...");
    for _ in 0..seconds {
        std::thread::sleep(Duration::from_secs(1));
        let stats = session.stats();
        println!(
            "frames {} duplicated {} dropped {} received {} {:.1} MB",
            stats.frames,
            stats.duplicated,
            stats.dropped,
            stats.frames_received,
            stats.bytes_written as f64 / 1_000_000.0
        );
        for track in &stats.tracks {
            println!(
                "  {}: peak {:.1} dBFS clipped {} packets {} overflows {} device lost {}",
                track.name, track.peak_dbfs, track.clipped, track.packets, track.overflows, track.device_lost
            );
        }
    }
    let final_stats = session.stats();
    println!("received {} frames from the capture", final_stats.frames_received);
    let summary = session.stop()?;
    println!(
        "file {} frames {} (expected {} = seconds x 60) duplicated {} dropped {} bytes {} duration {} ms average {:.2} Mbps",
        summary.file.display(),
        summary.frames,
        seconds * 60,
        summary.duplicated,
        summary.dropped,
        summary.bytes_written,
        summary.duration_ms,
        summary.average_mbps
    );
    for wav in &summary.wav_files {
        println!("wav {}", wav.display());
    }
    Ok(())
}
