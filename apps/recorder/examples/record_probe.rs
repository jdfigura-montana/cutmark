//! Records the primary monitor through the engine session, then prints the
//! totals. Usage: `cargo run -p cutmark-recorder --example record_probe
//! [seconds] [output.mkv]`

use std::path::PathBuf;
use std::time::Duration;

// The Recorder app has no lib target, so the example compiles the modules it
// needs straight from src; it uses only a subset of each.
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

use config::Profile;
use session::{RecordingSession, SessionConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let seconds: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(10);
    let output = args.next().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("record_probe.mkv"));

    let cfg = SessionConfig { adapter: 0, monitor: 0, cursor: true, profile: Profile::tutorial_av1_1440p60(), output_file: output };
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
    Ok(())
}
