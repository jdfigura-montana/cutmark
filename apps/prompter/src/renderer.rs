//! The wgpu setup: DX12 with DirectComposition presentation, so a view's surface can offer a
//! non-opaque alpha mode.

use std::sync::{Arc, OnceLock};

use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;

/// The composite alpha mode in use, filled in once the first surface exists.
pub type AlphaInUse = Arc<OnceLock<wgpu::CompositeAlphaMode>>;

/// The renderer setup. The adapter selector is the one place eframe hands out a real surface, so
/// the surface's alpha modes are read and logged there. It runs once.
pub fn wgpu_setup(alpha: AlphaInUse) -> WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    setup.instance_descriptor.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    let power = setup.power_preference;
    setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        // A selector replaces wgpu's own choice, so this ranks by device type the way wgpu does.
        let adapter = adapters
            .iter()
            .filter(|adapter| surface.is_none_or(|s| adapter.is_surface_supported(s)))
            .min_by_key(|adapter| rank(adapter.get_info().device_type, power))
            .ok_or_else(|| "no DX12 adapter supports the window surface".to_owned())?;
        if let Some(surface) = surface {
            let supported = surface.get_capabilities(adapter).alpha_modes;
            let used = pick_alpha_mode(&supported);
            log::info!("surface composite alpha modes: {supported:?}; in use for transparent windows: {used:?}");
            // A second run would be a new render state; the first answer stands.
            let _ = alpha.set(used);
        }
        Ok(adapter.clone())
    }));
    setup.into()
}

fn rank(device: wgpu::DeviceType, power: wgpu::PowerPreference) -> u8 {
    let low_power = power == wgpu::PowerPreference::LowPower;
    match device {
        wgpu::DeviceType::DiscreteGpu => u8::from(low_power),
        wgpu::DeviceType::IntegratedGpu => u8::from(!low_power),
        wgpu::DeviceType::Other => 2,
        wgpu::DeviceType::VirtualGpu => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

/// The rule egui-wgpu 0.36 applies when the root viewport is transparent. It does not expose its
/// choice, so this repeats it; check it again when egui-wgpu is upgraded.
fn pick_alpha_mode(supported: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    use wgpu::CompositeAlphaMode::{Auto, PostMultiplied, PreMultiplied};
    [PreMultiplied, PostMultiplied].into_iter().find(|mode| supported.contains(mode)).unwrap_or(Auto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::CompositeAlphaMode::{Auto, Opaque, PostMultiplied, PreMultiplied};

    #[test]
    fn premultiplied_wins_then_postmultiplied_then_auto() {
        assert_eq!(pick_alpha_mode(&[Opaque, PostMultiplied, PreMultiplied]), PreMultiplied);
        assert_eq!(pick_alpha_mode(&[Opaque, PostMultiplied]), PostMultiplied);
        assert_eq!(pick_alpha_mode(&[Opaque]), Auto);
    }
}
