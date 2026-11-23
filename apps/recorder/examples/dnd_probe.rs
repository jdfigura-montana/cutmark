//! Prints the Do Not Disturb state once a second for 30 seconds, for hand checks of the shell query.
//!
//! The Recorder is a binary crate, so the platform module is included by path.

// The platform module holds other Windows helpers that the probe never calls.
#[allow(dead_code, reason = "the probe reuses the recorder's own platform module for the one query it needs")]
#[path = "../src/platform.rs"]
mod platform;

use std::time::Duration;

/// Turns a raw QUERY_USER_NOTIFICATION_STATE value into its Windows name.
fn quns_name(raw: i32) -> &'static str {
    match raw {
        1 => "QUNS_NOT_PRESENT",
        2 => "QUNS_BUSY",
        3 => "QUNS_RUNNING_D3D_FULL_SCREEN",
        4 => "QUNS_PRESENTATION_MODE",
        5 => "QUNS_ACCEPTS_NOTIFICATIONS",
        6 => "QUNS_QUIET_TIME",
        7 => "QUNS_APP",
        _ => "no value",
    }
}

fn main() {
    for _ in 0..30 {
        let (state, raw) = platform::notification_state_raw();
        let time = chrono::Local::now().format("%H:%M:%S");
        println!("{time} QUNS {raw} ({}) state {state:?}", quns_name(raw));
        std::thread::sleep(Duration::from_secs(1));
    }
}
