//! The Chroma key section of the Webcam panel: key color, eyedropper, sliders, matte view and garbage matte.
//!
//! Every change goes straight into the active profile in the shared store, which saves it with its one debounced
//! write. The compose thread reads the profile's key on the next frame, so the preview follows the sliders live.

use egui::{Checkbox, Slider, Ui};

use crate::config::KeyConfig;
use crate::engine::controller::{ControllerState, Phase};
use crate::preview_ui::Pick;
use crate::profiles::ProfileStore;
use crate::webcam::WebcamHost;

/// The width of the sliders, in points.
const SLIDER_WIDTH: f32 = 220.0;

/// The size of the square the eyedropper averages, in preview pixels, on each side.
const EYEDROPPER_SIDE: u32 = 5;

/// What the Chroma key section remembers between frames; none of it is saved.
#[derive(Debug, Default)]
pub struct KeyUiState {
    /// Whether the preview shows the key as a black and white matte; it only works while idle.
    pub matte: bool,
    /// Whether the next click on the camera picture picks the key color.
    pub eyedropper: bool,
}

impl KeyUiState {
    /// Switches the matte view and the eyedropper off, for the moment a take starts or anything else has begun.
    pub fn clear(&mut self) {
        self.matte = false;
        self.eyedropper = false;
    }
}

/// True while nothing records, counts down, runs a pre-flight or runs a sync test, the only time the matte view may be on.
pub fn is_idle(state: &ControllerState) -> bool {
    state.phase == Phase::Idle && state.preflight.is_none() && state.sync_test.is_none()
}

/// Draws the Chroma key section; `picked` is the spot the user clicked on the camera picture this frame, if any.
///
/// The section is read-only while a take runs, because a take uses the profile as it was when it started.
pub fn show(ui: &mut Ui, host: &WebcamHost, store: &mut ProfileStore, state: &ControllerState, key_state: &mut KeyUiState, picked: Option<&Pick>) {
    ui.label("Chroma key");
    let Some(saved) = host.settings(store).map(|settings| settings.key.clone()) else { return };
    let idle = is_idle(state);
    let mut key = saved.clone();
    ui.add_enabled_ui(idle, |ui| {
        ui.checkbox(&mut key.enabled, "Enable key");
        ui.horizontal(|ui| {
            ui.label("Key color");
            ui.color_edit_button_srgb(&mut key.color);
            let label = if key_state.eyedropper { "Click the camera picture..." } else { "Eyedropper" };
            if ui.selectable_label(key_state.eyedropper, label).clicked() {
                key_state.eyedropper = !key_state.eyedropper;
            }
        });
        if key_state.eyedropper
            && let Some(pick) = picked
        {
            if let Some(color) = average_patch(&pick.frame.rgba, pick.frame.width, pick.frame.height, pick.x, pick.y) {
                key.color = color;
                key.enabled = true;
            }
            key_state.eyedropper = false;
        }
        ui.add_enabled(key.enabled, Checkbox::new(&mut key_state.matte, "Matte view"));
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = SLIDER_WIDTH;
            ui.add_enabled_ui(key.enabled, |ui| {
                ui.add(Slider::new(&mut key.similarity, 0.0..=1.0).text("Similarity"));
                ui.add(Slider::new(&mut key.smoothness, 0.0..=1.0).text("Smoothness"));
                ui.add(Slider::new(&mut key.spill, 0.0..=1.0).text("Spill"));
            });
            ui.label("Garbage crop");
            ui.add(Slider::new(&mut key.crop_left_pct, 0.0..=90.0).suffix("%").text("Left"));
            ui.add(Slider::new(&mut key.crop_right_pct, 0.0..=90.0).suffix("%").text("Right"));
            ui.add(Slider::new(&mut key.crop_top_pct, 0.0..=90.0).suffix("%").text("Top"));
            ui.add(Slider::new(&mut key.crop_bottom_pct, 0.0..=90.0).suffix("%").text("Bottom"));
            ui.checkbox(&mut key.mask_enabled, "Mask rectangle");
            ui.add_enabled_ui(key.mask_enabled, |ui| mask_sliders(ui, &mut key));
        });
    });
    if !idle {
        key_state.clear();
    }
    if key != saved {
        store.edit(&state.active_profile, |profile| {
            if let Some(settings) = &mut profile.webcam {
                settings.key = key.clone();
            }
        });
    }
}

/// The four sliders of the mask rectangle.
fn mask_sliders(ui: &mut Ui, key: &mut KeyConfig) {
    ui.add(Slider::new(&mut key.mask_x_pct, 0.0..=100.0).suffix("%").text("X"));
    ui.add(Slider::new(&mut key.mask_y_pct, 0.0..=100.0).suffix("%").text("Y"));
    ui.add(Slider::new(&mut key.mask_w_pct, 0.0..=100.0).suffix("%").text("Width"));
    ui.add(Slider::new(&mut key.mask_h_pct, 0.0..=100.0).suffix("%").text("Height"));
}

/// The average color of the 5x5 square of straight RGBA pixels around a point.
///
/// At the edge of the frame the square is cut off, so a corner averages the 3x3 pixels that exist. Returns None when
/// the frame has no pixels, the byte count does not match its size, or the point is outside it.
pub fn average_patch(rgba: &[u8], width: u32, height: u32, x: u32, y: u32) -> Option<[u8; 3]> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() != w * h * 4 || x >= width || y >= height {
        return None;
    }
    let reach = (EYEDROPPER_SIDE / 2) as usize;
    let (x, y) = (x as usize, y as usize);
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    for row in y.saturating_sub(reach)..=(y + reach).min(h - 1) {
        for column in x.saturating_sub(reach)..=(x + reach).min(w - 1) {
            let at = (row * w + column) * 4;
            let pixel = rgba.get(at..at + 3)?;
            for (total, &value) in sum.iter_mut().zip(pixel) {
                *total += u64::from(value);
            }
            count += 1;
        }
    }
    let average = |total: u64| ((total + count / 2) / count) as u8;
    Some([average(sum[0]), average(sum[1]), average(sum[2])])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame where every pixel is the same color.
    fn flat(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        (0..width * height).flat_map(|_| [color[0], color[1], color[2], 255]).collect()
    }

    /// Sets one pixel of a frame.
    fn paint(frame: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 3]) {
        let at = ((y * width + x) * 4) as usize;
        frame[at..at + 3].copy_from_slice(&color);
    }

    #[test]
    fn a_flat_frame_averages_to_its_color_everywhere() {
        let frame = flat(8, 6, [0, 177, 64]);
        for (x, y) in [(0, 0), (7, 0), (0, 5), (7, 5), (3, 3)] {
            assert_eq!(average_patch(&frame, 8, 6, x, y), Some([0, 177, 64]), "at {x},{y}");
        }
    }

    #[test]
    fn the_middle_averages_all_25_pixels() {
        let mut frame = flat(9, 9, [0, 0, 0]);
        // One white pixel in the 5x5 square around (4, 4) lifts the average by 255 / 25.
        paint(&mut frame, 9, 2, 2, [255, 255, 255]);
        assert_eq!(average_patch(&frame, 9, 9, 4, 4), Some([10, 10, 10]));
        // The same pixel is outside the square around (5, 5).
        assert_eq!(average_patch(&frame, 9, 9, 5, 5), Some([0, 0, 0]));
    }

    #[test]
    fn a_corner_averages_only_the_9_pixels_that_exist() {
        let mut frame = flat(9, 9, [0, 0, 0]);
        paint(&mut frame, 9, 2, 2, [90, 0, 0]);
        paint(&mut frame, 9, 3, 3, [255, 255, 255]);
        // Around (0, 0) the square covers x and y from 0 to 2: nine pixels, one of them (2, 2) painted.
        assert_eq!(average_patch(&frame, 9, 9, 0, 0), Some([10, 0, 0]));
    }

    #[test]
    fn an_edge_averages_the_15_pixels_that_exist() {
        let mut frame = flat(9, 9, [0, 0, 0]);
        paint(&mut frame, 9, 0, 4, [150, 150, 150]);
        // Around (0, 4) the square covers x from 0 to 2 and y from 2 to 6: fifteen pixels.
        assert_eq!(average_patch(&frame, 9, 9, 0, 4), Some([10, 10, 10]));
        // The far edge is cut the same way.
        let mut right = flat(9, 9, [0, 0, 0]);
        paint(&mut right, 9, 8, 8, [150, 150, 150]);
        assert_eq!(average_patch(&right, 9, 9, 8, 8), Some([17, 17, 17]));
    }

    #[test]
    fn a_frame_smaller_than_the_square_averages_all_of_it() {
        let mut frame = flat(2, 1, [0, 0, 0]);
        paint(&mut frame, 2, 1, 0, [200, 100, 50]);
        assert_eq!(average_patch(&frame, 2, 1, 0, 0), Some([100, 50, 25]));
    }

    #[test]
    fn a_point_outside_or_a_broken_frame_gives_nothing() {
        let frame = flat(4, 4, [1, 2, 3]);
        assert_eq!(average_patch(&frame, 4, 4, 4, 0), None);
        assert_eq!(average_patch(&frame, 4, 4, 0, 4), None);
        assert_eq!(average_patch(&frame[..frame.len() - 4], 4, 4, 0, 0), None);
        assert_eq!(average_patch(&[], 0, 0, 0, 0), None);
    }
}
