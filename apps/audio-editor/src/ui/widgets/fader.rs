//! ui::widgets::fader: the dB and pan readout formats of the track header sliders.

/// Formats a dB slider value, e.g. "-3.5 dB"; values below -60 read "-60.0 dB".
pub fn db_text(db: f32) -> String {
    format!("{:.1} dB", db.max(-60.0))
}

/// Formats a pan value: "L50", "C" or "R30".
pub fn pan_text(pan: f32) -> String {
    let amount = (pan.abs() * 100.0).round() as i32;
    if amount < 5 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L{amount}")
    } else {
        format!("R{amount}")
    }
}
