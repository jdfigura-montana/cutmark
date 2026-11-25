//! A hand run check of per-app capture against a real app.
//!
//! Set CUTMARK_APP_AUDIO_EXE to the executable of a running app that is making
//! sound (set CUTMARK_APP_AUDIO_MODE=exclude to hear everything else instead),
//! then run `cargo test -p capture --test app_loopback -- --ignored --nocapture`.

#![cfg(windows)]

use std::error::Error;
use std::time::{Duration, Instant};

use capture::{AppLoopback, AppTarget, LoopbackMode, list_audio_apps};

/// How long the app is captured.
const CAPTURE_FOR: Duration = Duration::from_secs(10);

#[test]
#[ignore = "needs a running app that makes sound, named in CUTMARK_APP_AUDIO_EXE"]
fn captures_ten_seconds_from_the_named_app() -> Result<(), Box<dyn Error>> {
    let exe = std::env::var("CUTMARK_APP_AUDIO_EXE").map_err(|_| "set CUTMARK_APP_AUDIO_EXE to a running app's exe, for example spotify.exe")?;
    let exclude = std::env::var("CUTMARK_APP_AUDIO_MODE").is_ok_and(|mode| mode.eq_ignore_ascii_case("exclude"));
    let mode = if exclude { LoopbackMode::Exclude } else { LoopbackMode::Include };
    for app in list_audio_apps(false)? {
        println!("audio app: {} ({}) pids {:?} active {}", app.exe, app.display_name, app.pids, app.active);
    }

    let target = AppTarget::new(&exe);
    let capture = AppLoopback::start(target.clone(), mode)?;
    println!("capturing {} in {mode:?} mode, {} of {} instance(s) excluded", target.exe(), capture.excluded_instances(), capture.total_instances());

    let started = Instant::now();
    let (mut packets, mut frames, mut discontinuities, mut peak) = (0u64, 0u64, 0u64, 0f32);
    let mut first_stamp = None;
    let mut last_end = i64::MIN;
    let mut backwards = 0u64;
    while started.elapsed() < CAPTURE_FOR && !capture.lost() {
        let Ok(packet) = capture.receiver().recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        assert_eq!((packet.sample_rate, packet.channels), (48_000, 2));
        assert_eq!(packet.samples.len(), packet.frames as usize * 2);
        packets += 1;
        frames += u64::from(packet.frames);
        discontinuities += u64::from(packet.discontinuity);
        peak = packet.samples.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
        first_stamp.get_or_insert(packet.qpc_100ns);
        // Up to 10 ms of timestamp jitter is expected from WASAPI.
        backwards += u64::from(packet.qpc_100ns < last_end.saturating_sub(100_000));
        last_end = packet.qpc_100ns + i64::from(packet.frames) * 10_000_000 / 48_000;
    }

    let span_s = first_stamp.map_or(0.0, |first| (last_end - first) as f64 / 10_000_000.0);
    println!("packets {packets}, audio {:.3} s over a {span_s:.3} s QPC span, peak {peak:.4}", frames as f64 / 48_000.0);
    println!("discontinuities {discontinuities}, stamps going backwards {backwards}, overflows {}, lost {}", capture.overflows(), capture.lost());
    capture.stop();
    assert!(packets > 0, "no packet arrived");
    assert!(peak > 0.0, "only silence arrived; was the app playing?");
    Ok(())
}
