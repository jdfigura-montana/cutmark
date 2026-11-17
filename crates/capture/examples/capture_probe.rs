//! Prints the adapters and monitors, then captures the primary monitor for five seconds
//! and reports capture statistics. Run with: cargo run -p capture --example capture_probe.

#[cfg(windows)]
mod probe {
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    use capture::{Gpu, MonitorCapture, list_adapters, list_monitors, qpc_frequency};

    /// Runs the five second capture probe.
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        println!("Adapters:");
        for adapter in list_adapters()? {
            println!(
                "  [{}] {} (vendor 0x{:04X}, LUID {:08X}-{:08X}, {} MiB dedicated)",
                adapter.index, adapter.name, adapter.vendor_id, adapter.luid_high as u32, adapter.luid_low, adapter.dedicated_vram_mb,
            );
        }
        println!("Monitors:");
        let monitors = list_monitors()?;
        for monitor in &monitors {
            println!(
                "  [{}] {} at ({}, {}) {}x{}{}",
                monitor.index,
                monitor.name,
                monitor.x,
                monitor.y,
                monitor.width,
                monitor.height,
                if monitor.primary { " (primary)" } else { "" },
            );
        }
        let gpu = Gpu::create(Some(0))?;
        println!("Device on adapter [{}]: {}", gpu.adapter().index, gpu.adapter().name,);
        let monitor = monitors.iter().find(|monitor| monitor.primary).ok_or("no primary monitor found")?;
        let capture = MonitorCapture::start(&gpu, monitor, true)?;
        println!("Capturing {} for 5 seconds with the cursor on...", monitor.name);
        let start = Instant::now();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut last_size = (0i32, 0i32);
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(frame) = capture.latest() {
                seen.insert(frame.seq);
                last_size = (frame.width, frame.height);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let elapsed = start.elapsed().as_secs_f64();
        let received = capture.frames_received();
        capture.stop();
        println!("Frames received: {received}");
        println!("Unique sequence numbers seen: {}", seen.len());
        println!("Average frames per second: {:.1}", f64::from(seen.len() as u32) / elapsed);
        println!("Frame size: {}x{}", last_size.0, last_size.1);
        println!("QPC frequency: {} ticks per second", qpc_frequency());
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    probe::run()
}

#[cfg(not(windows))]
fn main() {
    println!("capture_probe is only available on Windows");
}
