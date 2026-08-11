use bevy::{
    image::{ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::Skybox,
    prelude::*,
};
use bevy_panorbit_camera::PanOrbitCamera;

use crate::render_tuning::{
    ENVIRONMENT_DIFFUSE_PATH, ENVIRONMENT_LIGHT_INTENSITY, ENVIRONMENT_SPECULAR_PATH,
    NIGHT_4K_SKYBOX_PATH, NIGHT_8K_SKYBOX_PATH, NIGHT_ENVIRONMENT_DIFFUSE_PATH,
    NIGHT_ENVIRONMENT_LIGHT_INTENSITY, NIGHT_ENVIRONMENT_SPECULAR_PATH, NIGHT_SKY_BRIGHTNESS,
    SKYBOX_BRIGHTNESS, SKYBOX_PATH, environment_rotation, night_sky_rotation,
};

pub(crate) struct SkyboxPlugin;

#[cfg(target_arch = "wasm32")]
const GRAPHICS_PRESET_STORAGE_KEY: &str = "capablanca_chess.graphics_preset";

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GraphicsSelection>()
            .init_resource::<ActiveGraphicsPreset>()
            .add_systems(Startup, initialize_environment_assets)
            .add_systems(Update, apply_environment_maps);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GraphicsPreset {
    Low,
    Medium,
    Ultra,
}

#[derive(Resource, Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GraphicsSelection(pub(crate) GraphicsPreset);

impl Default for GraphicsSelection {
    fn default() -> Self {
        Self(load_graphics_preset().unwrap_or(GraphicsPreset::Ultra))
    }
}

#[derive(Resource, Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActiveGraphicsPreset(pub(crate) GraphicsPreset);

impl Default for ActiveGraphicsPreset {
    fn default() -> Self {
        // Always make the first page load cheap. The remembered selection is
        // activated only when the user starts a game.
        Self(GraphicsPreset::Low)
    }
}

impl GraphicsPreset {
    pub(crate) const fn slider_value(self) -> f32 {
        match self {
            Self::Low => 0.0,
            Self::Medium => 1.0,
            Self::Ultra => 2.0,
        }
    }

    pub(crate) fn from_slider_value(value: f32) -> Self {
        match value.round().clamp(0.0, 2.0) as u8 {
            0 => Self::Low,
            1 => Self::Medium,
            _ => Self::Ultra,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::Ultra => "ULTRA",
        }
    }
}

pub(crate) fn load_graphics_preset() -> Option<GraphicsPreset> {
    load_graphics_preset_impl()
}

pub(crate) fn save_graphics_preset(preset: GraphicsPreset) {
    save_graphics_preset_impl(preset);
}

#[cfg(not(target_arch = "wasm32"))]
fn load_graphics_preset_impl() -> Option<GraphicsPreset> {
    None
}

#[cfg(target_arch = "wasm32")]
fn load_graphics_preset_impl() -> Option<GraphicsPreset> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok())
        .flatten()
        .and_then(|storage| storage.get_item(GRAPHICS_PRESET_STORAGE_KEY).ok().flatten())
        .and_then(|value| parse_graphics_preset(&value))
}

#[cfg(not(target_arch = "wasm32"))]
fn save_graphics_preset_impl(_preset: GraphicsPreset) {}

#[cfg(target_arch = "wasm32")]
fn save_graphics_preset_impl(preset: GraphicsPreset) {
    let Some(storage) = web_sys::window()
        .and_then(|window| window.local_storage().ok())
        .flatten()
    else {
        return;
    };
    let _ = storage.set_item(
        GRAPHICS_PRESET_STORAGE_KEY,
        match preset {
            GraphicsPreset::Low => "low",
            GraphicsPreset::Medium => "medium",
            GraphicsPreset::Ultra => "ultra",
        },
    );
}

#[cfg(any(target_arch = "wasm32", test))]
fn parse_graphics_preset(value: &str) -> Option<GraphicsPreset> {
    match value {
        "low" | "night_4k" => Some(GraphicsPreset::Low),
        "medium" | "night_8k" => Some(GraphicsPreset::Medium),
        "ultra" | "nebula" => Some(GraphicsPreset::Ultra),
        _ => None,
    }
}

#[derive(Clone)]
struct EnvironmentSet {
    skybox: Handle<Image>,
    diffuse: Handle<Image>,
    specular: Handle<Image>,
    skybox_brightness: f32,
    light_intensity: f32,
}

#[derive(Resource)]
struct EnvironmentAssets {
    night_4k: Option<EnvironmentSet>,
    night_8k: Option<EnvironmentSet>,
    nebula: Option<EnvironmentSet>,
}

#[derive(Component)]
struct EnvironmentAttached(GraphicsPreset);

fn load_environment_set(
    asset_server: &AssetServer,
    skybox_path: &'static str,
    diffuse_path: &'static str,
    specular_path: &'static str,
    skybox_brightness: f32,
    light_intensity: f32,
) -> EnvironmentSet {
    let cubemap_sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let load_cubemap = |path: &'static str| {
        let sampler = cubemap_sampler.clone();
        asset_server
            .load_builder()
            .with_settings(move |settings: &mut ImageLoaderSettings| {
                settings.sampler = sampler.clone();
            })
            .load(path)
    };

    EnvironmentSet {
        skybox: load_cubemap(skybox_path),
        diffuse: load_cubemap(diffuse_path),
        specular: load_cubemap(specular_path),
        skybox_brightness,
        light_intensity,
    }
}

fn initialize_environment_assets(mut commands: Commands) {
    commands.insert_resource(EnvironmentAssets {
        night_4k: None,
        night_8k: None,
        nebula: None,
    });
}

fn apply_environment_maps(
    mut commands: Commands,
    active_preset: Res<ActiveGraphicsPreset>,
    asset_server: Res<AssetServer>,
    mut assets: ResMut<EnvironmentAssets>,
    cameras: Query<(Entity, Has<PanOrbitCamera>, Option<&EnvironmentAttached>), With<Camera3d>>,
) {
    let preset = active_preset.0;
    let rotation = match preset {
        GraphicsPreset::Low | GraphicsPreset::Medium => night_sky_rotation(),
        GraphicsPreset::Ultra => environment_rotation(),
    };
    let selected = match preset {
        GraphicsPreset::Low => {
            assets.night_8k = None;
            assets.nebula = None;
            assets
                .night_4k
                .get_or_insert_with(|| {
                    load_environment_set(
                        &asset_server,
                        NIGHT_4K_SKYBOX_PATH,
                        NIGHT_ENVIRONMENT_DIFFUSE_PATH,
                        NIGHT_ENVIRONMENT_SPECULAR_PATH,
                        NIGHT_SKY_BRIGHTNESS,
                        NIGHT_ENVIRONMENT_LIGHT_INTENSITY,
                    )
                })
                .clone()
        }
        GraphicsPreset::Medium => {
            assets.night_4k = None;
            assets.nebula = None;
            assets
                .night_8k
                .get_or_insert_with(|| {
                    load_environment_set(
                        &asset_server,
                        NIGHT_8K_SKYBOX_PATH,
                        NIGHT_ENVIRONMENT_DIFFUSE_PATH,
                        NIGHT_ENVIRONMENT_SPECULAR_PATH,
                        NIGHT_SKY_BRIGHTNESS,
                        NIGHT_ENVIRONMENT_LIGHT_INTENSITY,
                    )
                })
                .clone()
        }
        GraphicsPreset::Ultra => {
            assets.night_4k = None;
            assets.night_8k = None;
            assets
                .nebula
                .get_or_insert_with(|| {
                    load_environment_set(
                        &asset_server,
                        SKYBOX_PATH,
                        ENVIRONMENT_DIFFUSE_PATH,
                        ENVIRONMENT_SPECULAR_PATH,
                        SKYBOX_BRIGHTNESS,
                        ENVIRONMENT_LIGHT_INTENSITY,
                    )
                })
                .clone()
        }
    };

    for (entity, is_main_camera, attached) in &cameras {
        if attached.is_some_and(|attached| attached.0 == preset) {
            continue;
        }
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            EnvironmentMapLight {
                diffuse_map: selected.diffuse.clone(),
                specular_map: selected.specular.clone(),
                intensity: selected.light_intensity,
                rotation,
                ..default()
            },
            EnvironmentAttached(preset),
        ));
        if is_main_camera {
            entity_commands.insert(Skybox {
                image: Some(selected.skybox.clone()),
                brightness: selected.skybox_brightness,
                rotation,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_values_map_to_the_three_presets() {
        assert_eq!(GraphicsPreset::from_slider_value(-5.0), GraphicsPreset::Low);
        assert_eq!(GraphicsPreset::from_slider_value(0.49), GraphicsPreset::Low);
        assert_eq!(
            GraphicsPreset::from_slider_value(0.5),
            GraphicsPreset::Medium
        );
        assert_eq!(
            GraphicsPreset::from_slider_value(1.49),
            GraphicsPreset::Medium
        );
        assert_eq!(
            GraphicsPreset::from_slider_value(1.5),
            GraphicsPreset::Ultra
        );
        assert_eq!(
            GraphicsPreset::from_slider_value(8.0),
            GraphicsPreset::Ultra
        );
    }

    #[test]
    fn stored_preset_names_are_stable() {
        assert_eq!(parse_graphics_preset("low"), Some(GraphicsPreset::Low));
        assert_eq!(
            parse_graphics_preset("medium"),
            Some(GraphicsPreset::Medium)
        );
        assert_eq!(parse_graphics_preset("ultra"), Some(GraphicsPreset::Ultra));
        assert_eq!(parse_graphics_preset("night_4k"), Some(GraphicsPreset::Low));
        assert_eq!(
            parse_graphics_preset("night_8k"),
            Some(GraphicsPreset::Medium)
        );
        assert_eq!(parse_graphics_preset("nebula"), Some(GraphicsPreset::Ultra));
        assert_eq!(parse_graphics_preset("invalid"), None);
    }

    #[test]
    fn first_run_selects_ultra_but_activates_low() {
        assert_eq!(GraphicsSelection::default().0, GraphicsPreset::Ultra);
        assert_eq!(ActiveGraphicsPreset::default().0, GraphicsPreset::Low);
    }
}
