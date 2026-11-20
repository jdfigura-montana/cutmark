//! Camera controls (exposure, focus, white balance, gain, brightness) through the media source's
//! IAMCameraControl and IAMVideoProcAmp interfaces.
//!
//! A property the camera does not offer is simply absent from the results; nothing here fails because a camera lacks a control.
#![allow(unsafe_code)]

use std::sync::Arc;

use super::MfGuard;
use crate::{CaptureError, with_context};
use windows::Win32::Media::DirectShow::{IAMCameraControl, IAMVideoProcAmp};
use windows::Win32::Media::MediaFoundation::IMFMediaSource;
use windows::core::Interface;

/// IAMCameraControl property id of exposure (CameraControl_Exposure).
const CAMERA_EXPOSURE: i32 = 4;
/// IAMCameraControl property id of focus (CameraControl_Focus).
const CAMERA_FOCUS: i32 = 6;
/// IAMVideoProcAmp property id of brightness (VideoProcAmp_Brightness).
const PROC_BRIGHTNESS: i32 = 0;
/// IAMVideoProcAmp property id of white balance (VideoProcAmp_WhiteBalance).
const PROC_WHITE_BALANCE: i32 = 7;
/// IAMVideoProcAmp property id of gain (VideoProcAmp_Gain).
const PROC_GAIN: i32 = 9;
/// The automatic flag of both interfaces.
const FLAG_AUTO: i32 = 1;
/// The manual flag of both interfaces.
const FLAG_MANUAL: i32 = 2;

/// A camera property the recorder can adjust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ControlProp {
    /// Exposure time.
    Exposure,
    /// Focus distance.
    Focus,
    /// White balance temperature.
    WhiteBalance,
    /// Sensor gain.
    Gain,
    /// Picture brightness.
    Brightness,
}

impl ControlProp {
    /// Every property, in the order the window lists them.
    pub const ALL: [Self; 5] = [Self::Exposure, Self::Focus, Self::WhiteBalance, Self::Gain, Self::Brightness];

    /// The name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Exposure => "Exposure",
            Self::Focus => "Focus",
            Self::WhiteBalance => "White balance",
            Self::Gain => "Gain",
            Self::Brightness => "Brightness",
        }
    }

    /// The DirectShow property id inside its interface.
    fn id(self) -> i32 {
        match self {
            Self::Exposure => CAMERA_EXPOSURE,
            Self::Focus => CAMERA_FOCUS,
            Self::WhiteBalance => PROC_WHITE_BALANCE,
            Self::Gain => PROC_GAIN,
            Self::Brightness => PROC_BRIGHTNESS,
        }
    }

    /// True for the properties of IAMCameraControl; the others belong to IAMVideoProcAmp.
    fn on_camera_control(self) -> bool {
        matches!(self, Self::Exposure | Self::Focus)
    }
}

/// What a camera allows for one property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlRange {
    /// The smallest value.
    pub min: i32,
    /// The largest value.
    pub max: i32,
    /// The distance between two allowed values.
    pub step: i32,
    /// The value the camera starts with.
    pub default: i32,
    /// Whether the camera can adjust the property by itself.
    pub auto_capable: bool,
}

impl ControlRange {
    /// The nearest allowed value to the given one.
    pub fn snap(&self, value: i32) -> i32 {
        let (min, max) = (self.min.min(self.max), self.min.max(self.max));
        let clamped = value.clamp(min, max);
        if self.step <= 1 {
            return clamped;
        }
        let step = i64::from(self.step);
        let steps = (i64::from(clamped) - i64::from(min) + step / 2) / step;
        let snapped = (i64::from(min) + steps * step).min(i64::from(max));
        i32::try_from(snapped).unwrap_or(clamped)
    }
}

/// The current value of one property and whether the camera adjusts it by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlState {
    /// The current value.
    pub value: i32,
    /// True while the camera adjusts the property by itself.
    pub auto: bool,
}

/// One supported property with its range and its current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlInfo {
    /// Which property this is.
    pub prop: ControlProp,
    /// What the camera allows.
    pub range: ControlRange,
    /// What the camera has now.
    pub current: ControlState,
}

/// The properties that are in auto mode, with the value to lock them at.
fn lock_targets(states: &[(ControlProp, ControlState)]) -> Vec<(ControlProp, i32)> {
    states.iter().filter(|(_, state)| state.auto).map(|(prop, state)| (*prop, state.value)).collect()
}

/// The value and auto flag every property returns to on a reset.
fn default_targets(ranges: &[(ControlProp, ControlRange)]) -> Vec<(ControlProp, ControlState)> {
    ranges.iter().map(|(prop, range)| (*prop, ControlState { value: range.default, auto: range.auto_capable })).collect()
}

/// The control interfaces of one camera; cheap to clone, and usable from any thread.
#[derive(Clone)]
pub struct CameraControls {
    camera: Option<IAMCameraControl>,
    proc_amp: Option<IAMVideoProcAmp>,
    /// Every supported property with its range, read once when the camera opened.
    ranges: Arc<Vec<(ControlProp, ControlRange)>>,
    /// Keeps Media Foundation started while the interfaces are alive.
    _mf: Option<Arc<MfGuard>>,
}

// SAFETY: the interfaces are queried from a Media Foundation device source, which is free-threaded
// (it lives in the multithreaded apartment), so any one thread at a time may call them.
unsafe impl Send for CameraControls {}

impl CameraControls {
    /// A set of controls with no properties, for a camera that is not open.
    pub fn empty() -> Self {
        Self { camera: None, proc_amp: None, ranges: Arc::new(Vec::new()), _mf: None }
    }

    /// Queries the control interfaces of a camera media source and reads the range of every property it offers.
    pub(crate) fn from_source(source: &IMFMediaSource) -> Self {
        let mut controls = Self {
            camera: source.cast::<IAMCameraControl>().ok(),
            proc_amp: source.cast::<IAMVideoProcAmp>().ok(),
            ranges: Arc::new(Vec::new()),
            _mf: MfGuard::acquire().ok().map(Arc::new),
        };
        let ranges: Vec<(ControlProp, ControlRange)> =
            ControlProp::ALL.iter().filter_map(|prop| controls.read_range(*prop).map(|range| (*prop, range))).collect();
        controls.ranges = Arc::new(ranges);
        controls
    }

    /// The range of a property, or None when the camera does not offer it.
    pub fn range(&self, prop: ControlProp) -> Option<ControlRange> {
        self.ranges.iter().find(|(known, _)| *known == prop).map(|(_, range)| *range)
    }

    /// Every supported property with its current state.
    pub fn supported(&self) -> Vec<ControlInfo> {
        self.ranges.iter().filter_map(|(prop, range)| self.current(*prop).map(|current| ControlInfo { prop: *prop, range: *range, current })).collect()
    }

    /// The current state of a property, or None when the camera does not offer it.
    pub fn current(&self, prop: ControlProp) -> Option<ControlState> {
        self.range(prop)?;
        let (mut value, mut flags) = (0i32, 0i32);
        let result = if prop.on_camera_control() {
            let camera = self.camera.as_ref()?;
            // SAFETY: the interface is live and both out pointers are valid for the call.
            unsafe { camera.Get(prop.id(), &mut value, &mut flags) }
        } else {
            let proc_amp = self.proc_amp.as_ref()?;
            // SAFETY: the interface is live and both out pointers are valid for the call.
            unsafe { proc_amp.Get(prop.id(), &mut value, &mut flags) }
        };
        result.ok()?;
        Some(ControlState { value, auto: flags & FLAG_AUTO != 0 })
    }

    /// Sets a property to a value (snapped to the camera's step) in manual mode, or hands it to the camera with `auto`.
    pub fn set(&self, prop: ControlProp, value: i32, auto: bool) -> Result<(), CaptureError> {
        let range = self.range(prop).ok_or(CaptureError::Missing("the camera does not offer this control"))?;
        if auto && !range.auto_capable {
            return Err(CaptureError::Missing("this control has no automatic mode"));
        }
        let value = range.snap(value);
        let flags = if auto { FLAG_AUTO } else { FLAG_MANUAL };
        if prop.on_camera_control() {
            let camera = self.camera.as_ref().ok_or(CaptureError::Missing("the camera has no camera control interface"))?;
            // SAFETY: the interface is live and the call only takes plain values.
            with_context("failed to set the camera control", unsafe { camera.Set(prop.id(), value, flags) })
        } else {
            let proc_amp = self.proc_amp.as_ref().ok_or(CaptureError::Missing("the camera has no video processing control interface"))?;
            // SAFETY: the interface is live and the call only takes plain values.
            with_context("failed to set the camera control", unsafe { proc_amp.Set(prop.id(), value, flags) })
        }
    }

    /// Switches every property that is in auto mode to manual at its current value; returns what was locked.
    pub fn lock_current(&self) -> Vec<(ControlProp, ControlState)> {
        let states: Vec<(ControlProp, ControlState)> = self.ranges.iter().filter_map(|(prop, _)| self.current(*prop).map(|state| (*prop, state))).collect();
        let mut locked = Vec::new();
        for (prop, value) in lock_targets(&states) {
            match self.set(prop, value, false) {
                Ok(()) => locked.push((prop, ControlState { value, auto: false })),
                Err(error) => log::warn!("webcam: could not lock {}: {error}", prop.label()),
            }
        }
        locked
    }

    /// Returns every property to the camera's default value, with automatic mode on where the camera has one.
    pub fn reset_defaults(&self) -> Vec<(ControlProp, ControlState)> {
        let mut applied = Vec::new();
        for (prop, state) in default_targets(&self.ranges) {
            match self.set(prop, state.value, state.auto) {
                Ok(()) => applied.push((prop, state)),
                Err(error) => log::warn!("webcam: could not reset {}: {error}", prop.label()),
            }
        }
        applied
    }

    /// Reads the range of a property from the camera; None when it is not offered or has nothing to adjust.
    fn read_range(&self, prop: ControlProp) -> Option<ControlRange> {
        let (mut min, mut max, mut step, mut default, mut caps) = (0i32, 0i32, 0i32, 0i32, 0i32);
        let result = if prop.on_camera_control() {
            let camera = self.camera.as_ref()?;
            // SAFETY: the interface is live and every out pointer is valid for the call.
            unsafe { camera.GetRange(prop.id(), &mut min, &mut max, &mut step, &mut default, &mut caps) }
        } else {
            let proc_amp = self.proc_amp.as_ref()?;
            // SAFETY: the interface is live and every out pointer is valid for the call.
            unsafe { proc_amp.GetRange(prop.id(), &mut min, &mut max, &mut step, &mut default, &mut caps) }
        };
        result.ok()?;
        if max <= min {
            return None;
        }
        Some(ControlRange { min, max, step, default, auto_capable: caps & FLAG_AUTO != 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(min: i32, max: i32, step: i32) -> ControlRange {
        ControlRange { min, max, step, default: min, auto_capable: true }
    }

    #[test]
    fn snap_clamps_to_the_range() {
        let r = range(0, 255, 1);
        assert_eq!(r.snap(-5), 0);
        assert_eq!(r.snap(300), 255);
        assert_eq!(r.snap(100), 100);
    }

    #[test]
    fn snap_rounds_to_the_nearest_step_from_the_minimum() {
        let r = range(2, 22, 5);
        assert_eq!(r.snap(2), 2);
        assert_eq!(r.snap(4), 2);
        assert_eq!(r.snap(5), 7);
        assert_eq!(r.snap(100), 22);
        // 21 is nearer to 22 than to 17.
        assert_eq!(r.snap(21), 22);
    }

    #[test]
    fn snap_survives_a_backwards_range() {
        let r = range(10, 0, 1);
        assert_eq!(r.snap(5), 5);
    }

    #[test]
    fn lock_targets_pick_only_the_auto_properties() {
        let states = [
            (ControlProp::Exposure, ControlState { value: -6, auto: true }),
            (ControlProp::Gain, ControlState { value: 10, auto: false }),
            (ControlProp::WhiteBalance, ControlState { value: 4600, auto: true }),
        ];
        assert_eq!(lock_targets(&states), vec![(ControlProp::Exposure, -6), (ControlProp::WhiteBalance, 4600)]);
        assert!(lock_targets(&[]).is_empty());
    }

    #[test]
    fn default_targets_use_the_default_and_auto_where_possible() {
        let with_auto = ControlRange { min: -10, max: 0, step: 1, default: -5, auto_capable: true };
        let manual_only = ControlRange { min: 0, max: 100, step: 1, default: 40, auto_capable: false };
        let targets = default_targets(&[(ControlProp::Exposure, with_auto), (ControlProp::Gain, manual_only)]);
        assert_eq!(
            targets,
            vec![(ControlProp::Exposure, ControlState { value: -5, auto: true }), (ControlProp::Gain, ControlState { value: 40, auto: false })]
        );
    }

    #[test]
    fn an_empty_set_of_controls_has_nothing_and_never_errors() {
        let controls = CameraControls::empty();
        assert!(controls.supported().is_empty());
        assert!(controls.current(ControlProp::Exposure).is_none());
        assert!(controls.lock_current().is_empty());
        assert!(controls.reset_defaults().is_empty());
        assert!(controls.set(ControlProp::Gain, 1, false).is_err());
    }

    #[test]
    fn every_property_has_its_own_id_and_label() {
        let mut ids: Vec<(bool, i32)> = ControlProp::ALL.iter().map(|prop| (prop.on_camera_control(), prop.id())).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ControlProp::ALL.len());
        assert_eq!(ControlProp::WhiteBalance.label(), "White balance");
    }
}
