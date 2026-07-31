use bevy::{camera::visibility::RenderLayers, prelude::*};
use bevy_panorbit_camera::PanOrbitCamera;

pub(crate) struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_environment);
    }
}

fn setup_environment(mut commands: Commands) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.78, 0.84, 1.0),
        brightness: 260.0,
        ..default()
    });

    let light_transform =
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.55, 0.0));
    commands.spawn((
        DirectionalLight {
            illuminance: 9_500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        light_transform,
        RenderLayers::layer(0),
    ));
    // The reflection pass gets equivalent lighting without a second set of
    // directional shadow maps. The reflected pieces are small enough that the
    // visual difference is negligible, while this matters on web GPUs.
    commands.spawn((
        DirectionalLight {
            illuminance: 9_500.0,
            shadow_maps_enabled: false,
            ..default()
        },
        light_transform,
        RenderLayers::layer(1),
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 10.5, -13.5).looking_at(Vec3::ZERO, Vec3::Y),
        PanOrbitCamera {
            button_orbit: MouseButton::Middle,
            button_pan: MouseButton::Right,
            zoom_lower_limit: 6.0,
            zoom_upper_limit: Some(25.0),
            ..default()
        },
    ));
}
