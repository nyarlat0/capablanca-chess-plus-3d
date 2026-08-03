//! Centralized visual-quality controls for the browser renderer.
//!
//! These values deliberately live outside the scene setup so visual tuning does
//! not require hunting through camera, skybox, and light spawning code.

use bevy::prelude::*;

// Skybox and image-based lighting share this rotation. Y is Bevy's height axis
// and the rotations are applied in XYZ Euler order.
pub(crate) const ENVIRONMENT_ROTATION_X_DEGREES: f32 = 30.0;
pub(crate) const ENVIRONMENT_ROTATION_Y_DEGREES: f32 = 220.0;
pub(crate) const ENVIRONMENT_ROTATION_Z_DEGREES: f32 = 0.0;
pub(crate) const SKYBOX_BRIGHTNESS: f32 = 700.0;
pub(crate) const ENVIRONMENT_LIGHT_INTENSITY: f32 = 420.0;
pub(crate) const TEXTURE_ANISOTROPY: u16 = 8;
// Positive bias selects a fractionally softer mip for PBR surface textures.
// This suppresses sub-pixel marble/wood/normal-map shimmer during camera motion
// without downscaling the scene or blurring geometry and UI.
pub(crate) const MATERIAL_TEXTURE_MIP_BIAS: f32 = 0.4;

#[cfg(target_arch = "wasm32")]
pub(crate) const SKYBOX_PATH: &str = "textures/generated/space_skybox.ktx2";
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SKYBOX_PATH: &str = "textures/generated/space_skybox.native.ktx2";
pub(crate) const ENVIRONMENT_DIFFUSE_PATH: &str = "textures/generated/space_diffuse.ktx2";
pub(crate) const ENVIRONMENT_SPECULAR_PATH: &str = "textures/generated/space_specular.ktx2";

pub(crate) fn generated_surface_texture_path(stem: &str) -> String {
    let target_suffix = if cfg!(target_arch = "wasm32") {
        ""
    } else {
        ".native"
    };
    format!("textures/generated/{stem}{target_suffix}.ktx2")
}

// The cubemap supplies most indirect light; global ambient is only a fallback
// for asset-loading frames and the deepest shadows.
pub(crate) const AMBIENT_LIGHT_COLOR: Color = Color::srgb(0.42, 0.36, 0.62);
pub(crate) const AMBIENT_LIGHT_BRIGHTNESS: f32 = 28.0;

pub(crate) const NEBULA_KEY_COLOR: Color = Color::srgb(1.0, 0.82, 0.9);
pub(crate) const NEBULA_KEY_ILLUMINANCE: f32 = 8_000.0;
pub(crate) const COSMIC_FILL_COLOR: Color = Color::srgb(0.42, 0.56, 1.0);
pub(crate) const COSMIC_FILL_ILLUMINANCE: f32 = 2_400.0;

// A weak, shadowless product-light which follows the camera. Unlike ambient
// light, it preserves the models' form; unlike another fixed world light, it
// cannot leave the camera-facing half of dark pieces unreadable after orbiting.
pub(crate) const CAMERA_FILL_COLOR: Color = Color::srgb(0.78, 0.72, 1.0);
pub(crate) const CAMERA_FILL_ILLUMINANCE: f32 = 1_000.0;
pub(crate) const CAMERA_FILL_PITCH_DEGREES: f32 = -8.0;
pub(crate) const CAMERA_FILL_YAW_DEGREES: f32 = 18.0;

// Black pieces remain recognizably black, but their warm albedo is deliberately
// separated from the cooler black marble. A broader base highlight plus a thin
// clearcoat keeps carved details visible without metallic or emissive cheating.
pub(crate) const BLACK_PIECE_COLOR: Color = Color::srgb(0.13, 0.085, 0.07);
pub(crate) const BLACK_PIECE_ROUGHNESS: f32 = 0.38;
pub(crate) const BLACK_PIECE_REFLECTANCE: f32 = 0.5;
pub(crate) const BLACK_PIECE_CLEARCOAT: f32 = 0.18;
pub(crate) const BLACK_PIECE_CLEARCOAT_ROUGHNESS: f32 = 0.22;

// A single tightly bounded cascade has much higher texel density than Bevy's
// general-purpose 150-unit default and matches WebGL2's one-cascade limit.
pub(crate) const DIRECTIONAL_SHADOW_MAP_SIZE: usize = 2_048;
pub(crate) const SHADOW_MINIMUM_DISTANCE: f32 = 0.1;
pub(crate) const SHADOW_MAXIMUM_DISTANCE: f32 = 32.0;

// Bloom is deliberately thresholded: only luminous stars, nebula cores and
// the strongest material highlights produce a halo, rather than softening the
// entire frame. These values operate on the exposed HDR image, before tone map.
pub(crate) const BLOOM_INTENSITY: f32 = 0.1;
pub(crate) const BLOOM_LOW_FREQUENCY_BOOST: f32 = 0.4;
pub(crate) const BLOOM_HIGH_PASS_FREQUENCY: f32 = 0.9;
pub(crate) const BLOOM_THRESHOLD: f32 = 0.5;
pub(crate) const BLOOM_THRESHOLD_SOFTNESS: f32 = 0.55;
pub(crate) const BLOOM_MAX_MIP_DIMENSION: u32 = 256;

pub(crate) const COLOR_GRADING_EXPOSURE: f32 = 0.0;
pub(crate) const COLOR_GRADING_TINT: f32 = 0.006;
pub(crate) const COLOR_GRADING_SATURATION: f32 = 1.05;

pub(crate) const VIGNETTE_INTENSITY: f32 = 0.12;
pub(crate) const VIGNETTE_RADIUS: f32 = 0.85;
pub(crate) const VIGNETTE_SMOOTHNESS: f32 = 5.0;

pub(crate) fn environment_rotation() -> Quat {
    Quat::from_euler(
        EulerRot::XYZ,
        ENVIRONMENT_ROTATION_X_DEGREES.to_radians(),
        ENVIRONMENT_ROTATION_Y_DEGREES.to_radians(),
        ENVIRONMENT_ROTATION_Z_DEGREES.to_radians(),
    )
}
