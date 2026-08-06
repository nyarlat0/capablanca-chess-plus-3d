use bevy::{
    image::{ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::Skybox,
    prelude::*,
};
use bevy_panorbit_camera::PanOrbitCamera;

use crate::{
    render_tuning::{
        ENVIRONMENT_DIFFUSE_PATH, ENVIRONMENT_LIGHT_INTENSITY, ENVIRONMENT_SPECULAR_PATH,
        SKYBOX_BRIGHTNESS, SKYBOX_PATH, environment_rotation,
    },
    settings::GraphicsSettings,
};

pub(crate) struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_environment_maps)
            .add_systems(Update, attach_environment_maps);
    }
}

#[derive(Resource)]
struct EnvironmentAssets {
    skybox: Option<Handle<Image>>,
    diffuse: Handle<Image>,
    specular: Handle<Image>,
}

#[derive(Component)]
struct EnvironmentAttached;

fn load_environment_maps(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    graphics: Res<GraphicsSettings>,
) {
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

    commands.insert_resource(EnvironmentAssets {
        skybox: (!graphics.low_end_mode).then(|| load_cubemap(SKYBOX_PATH)),
        diffuse: load_cubemap(ENVIRONMENT_DIFFUSE_PATH),
        specular: load_cubemap(ENVIRONMENT_SPECULAR_PATH),
    });
}

fn attach_environment_maps(
    mut commands: Commands,
    assets: Res<EnvironmentAssets>,
    cameras: Query<(Entity, Has<PanOrbitCamera>), (With<Camera3d>, Without<EnvironmentAttached>)>,
) {
    for (entity, is_main_camera) in &cameras {
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            EnvironmentMapLight {
                diffuse_map: assets.diffuse.clone(),
                specular_map: assets.specular.clone(),
                intensity: ENVIRONMENT_LIGHT_INTENSITY,
                rotation: environment_rotation(),
                ..default()
            },
            EnvironmentAttached,
        ));
        if is_main_camera && let Some(skybox) = &assets.skybox {
            entity_commands.insert(Skybox {
                image: Some(skybox.clone()),
                brightness: SKYBOX_BRIGHTNESS,
                rotation: environment_rotation(),
            });
        }
    }
}
