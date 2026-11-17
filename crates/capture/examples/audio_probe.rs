//! Lists audio devices, then captures the default loopback and mic for 5 seconds.
//!
//! Run with: cargo run -p capture --example audio_probe

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use capture::{AudioCapture, AudioDevice, AudioPacket, DeviceKind, list_audio_devices};

fn main() {
    let devices = list_audio_devices().expect("failed to list audio devices");
    println!("audio devices: {}", devices.len());
    for d in &devices {
        println!("  {:?} default={} rate={} ch={} \"{}\" {}", d.kind, d.is_default, d.sample_rate, d.channels, d.name, d.id);
    }
    let output = devices.iter().find(|d| d.kind == DeviceKind::Output && d.is_default).cloned().expect("no default output device");
    let input = devices.iter().find(|d| d.kind == DeviceKind::Input && d.is_default).cloned().expect("no default input device");

    let captures = vec![
        ("loopback", output.clone(), AudioCapture::start(&output).expect("failed to start loopback capture")),
        ("input", input.clone(), AudioCapture::start(&input).expect("failed to start input capture")),
    ];

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut collected: Vec<Vec<AudioPacket>> = vec![Vec::new(), Vec::new()];
    while Instant::now() < deadline {
        for (i, (_, _, capture)) in captures.iter().enumerate() {
            let timeout = deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(200));
            if let Ok(packet) = capture.receiver().recv_timeout(timeout) {
                collected[i].push(packet);
            }
        }
    }
    for (i, (label, device, _)) in captures.iter().enumerate() {
        report(label, device, &collected[i]);
    }
    // Dropping stops both captures and joins their threads.
    drop(captures);
}

/// Prints the capture statistics for one stream.
fn report(label: &str, device: &AudioDevice, packets: &[AudioPacket]) {
    let total_frames: u64 = packets.iter().map(|p| p.frames as u64).sum();
    let peak = packets.iter().flat_map(|p| p.samples.iter()).fold(0.0f32, |m, s| m.max(s.abs()));
    let peak_db = if peak > 0.0 { 20.0 * peak.log10() } else { f32::NEG_INFINITY };
    let fps = f64::from(total_frames as u32) / 5.0;
    let max_gap_ms = packets.windows(2).map(|w| (w[1].qpc_100ns - w[0].qpc_100ns).max(0)).max().map_or(0.0, |gap| f64::from(gap as u32) / 10_000.0);
    let first = packets.first().map_or(0, |p| p.qpc_100ns);
    let last = packets.last().map_or(0, |p| p.qpc_100ns);
    let span_ms = f64::from((last - first).max(0) as u32) / 10_000.0;
    let frames_ms = total_frames as f64 / f64::from(device.sample_rate) * 1000.0;
    let drift_ms = span_ms - frames_ms;
    let name = device.name.as_str();
    println!(
        "{label}: \"{name}\" packets={} frames={} ({fps:.1} fps) peak={peak_db:.1} dBFS max gap={max_gap_ms:.2} ms packet clock drift={drift_ms:+.2} ms",
        packets.len(),
        total_frames,
    );
}
