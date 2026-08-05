use std::f32::consts::{PI, TAU};

use bevy::{
    anti_alias::smaa::{Smaa, SmaaPreset},
    camera::{Hdr, visibility::RenderLayers},
    core_pipeline::tonemapping::Tonemapping,
    light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap},
    post_process::{
        bloom::{Bloom, BloomCompositeMode, BloomPrefilter},
        effect_stack::Vignette,
    },
    prelude::*,
    render::camera::MipBias,
    render::view::{ColorGrading, ColorGradingGlobal},
};
use bevy_panorbit_camera::{PanOrbitCamera, TouchControls};
use capablanca_chess_plus::Color as Side;

use crate::{
    app::FrontendSet,
    board::board_camera_radius,
    game::ChessMatch,
    menu::{GameMenuState, GameMode},
    pieces::PieceAnimationState,
    render_tuning::{
        AMBIENT_LIGHT_BRIGHTNESS, AMBIENT_LIGHT_COLOR, BLOOM_HIGH_PASS_FREQUENCY, BLOOM_INTENSITY,
        BLOOM_LOW_FREQUENCY_BOOST, BLOOM_MAX_MIP_DIMENSION, BLOOM_THRESHOLD,
        BLOOM_THRESHOLD_SOFTNESS, CAMERA_FILL_COLOR, CAMERA_FILL_ILLUMINANCE,
        CAMERA_FILL_PITCH_DEGREES, CAMERA_FILL_YAW_DEGREES, COLOR_GRADING_EXPOSURE,
        COLOR_GRADING_SATURATION, COLOR_GRADING_TINT, COSMIC_FILL_COLOR, COSMIC_FILL_ILLUMINANCE,
        DIRECTIONAL_SHADOW_MAP_SIZE, MATERIAL_TEXTURE_MIP_BIAS, NEBULA_KEY_COLOR,
        NEBULA_KEY_ILLUMINANCE, SHADOW_MAXIMUM_DISTANCE, SHADOW_MINIMUM_DISTANCE,
        VIGNETTE_INTENSITY, VIGNETTE_RADIUS, VIGNETTE_SMOOTHNESS, WEB_CAMERA_FILL_MIN_RANGE,
        WEB_CAMERA_FILL_RADIUS, WEB_CAMERA_FILL_RANGE_MULTIPLIER, WEB_COSMIC_FILL_POSITION,
        WEB_COSMIC_FILL_RADIUS, WEB_COSMIC_FILL_RANGE,
    },
};

// Duration of the automatic half-turn after a move or starting a game.
// This does not affect mouse-controlled camera movement.
const AUTO_TURN_SECONDS: f32 = 1.4;
// Recentring should finish sooner than the turn, but still ease in and out.
const AUTO_RECENTER_SECONDS: f32 = 0.5;
// Canonical elevation of the original camera at (0, 10.5, -13.5).
const HOME_CAMERA_PITCH_RADIANS: f32 = 0.661_043;
const HOME_CAMERA_RADIUS: f32 = 17.102_63;
// Manual orbit only. The first two project commits relied on the library's
// 1.0 sensitivity / 0.1 smoothness defaults; with left-button dragging that
// feels abrupt, so keep the same controller but soften its response explicitly.
const MANUAL_ORBIT_SENSITIVITY: f32 = 0.82;
const MANUAL_ORBIT_SMOOTHNESS: f32 = 0.35;

pub(crate) struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::BLACK))
            .insert_resource(DirectionalLightShadowMap {
                size: DIRECTIONAL_SHADOW_MAP_SIZE,
            })
            .init_resource::<LocalCameraState>()
            .init_resource::<CameraAutoTurn>()
            .add_systems(Startup, setup_environment)
            .add_systems(
                Update,
                (
                    orient_camera_after_local_move,
                    handle_manual_camera_recenter,
                    animate_automatic_camera_turn,
                )
                    .chain()
                    .in_set(FrontendSet::Camera),
            );

        #[cfg(target_arch = "wasm32")]
        app.add_systems(Update, update_web_camera_fill.in_set(FrontendSet::Camera));
    }
}

#[derive(Resource, Default)]
pub(crate) struct CameraAutoTurn {
    active: Option<AutomaticCameraTurn>,
}

struct AutomaticCameraTurn {
    start_yaw: f32,
    end_yaw: f32,
    start_focus: Vec3,
    full_recenter: Option<FullCameraRecenter>,
    elapsed: f32,
}

struct FullCameraRecenter {
    start_pitch: f32,
    end_pitch: f32,
    start_radius: f32,
    end_radius: f32,
}

#[derive(Resource)]
struct LocalCameraState {
    observed_generation: u64,
}

impl Default for LocalCameraState {
    fn default() -> Self {
        Self {
            observed_generation: u64::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GenerationCameraAction {
    Wait,
    Observe,
    Orient(Side),
}

#[derive(Component)]
struct WebCameraReadabilityFill;

fn setup_environment(mut commands: Commands) {
    commands.insert_resource(GlobalAmbientLight {
        color: AMBIENT_LIGHT_COLOR,
        brightness: AMBIENT_LIGHT_BRIGHTNESS,
        ..default()
    });

    let key_light_transform =
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.55, 0.0));
    commands.spawn((
        DirectionalLight {
            color: NEBULA_KEY_COLOR,
            illuminance: NEBULA_KEY_ILLUMINANCE,
            shadow_maps_enabled: true,
            ..default()
        },
        key_light_transform,
        CascadeShadowConfigBuilder {
            num_cascades: 1,
            minimum_distance: SHADOW_MINIMUM_DISTANCE,
            maximum_distance: SHADOW_MAXIMUM_DISTANCE,
            ..default()
        }
        .build(),
        if cfg!(target_arch = "wasm32") {
            // WebGL2 exposes one directional light. Let that one key light
            // serve both the main and planar-reflection cameras.
            RenderLayers::layer(0).with(1)
        } else {
            RenderLayers::layer(0)
        },
        Name::new("Nebula key light"),
    ));

    if cfg!(target_arch = "wasm32") {
        // Clustered point lights are supported by the WebGL2 CPU fallback and
        // do not consume its single directional-light slot. This broad fixed
        // fill keeps the cool opposing color in both views.
        commands.spawn((
            PointLight {
                color: COSMIC_FILL_COLOR,
                intensity: point_light_lumens(
                    COSMIC_FILL_ILLUMINANCE,
                    WEB_COSMIC_FILL_POSITION.length(),
                ),
                range: WEB_COSMIC_FILL_RANGE,
                radius: WEB_COSMIC_FILL_RADIUS,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(WEB_COSMIC_FILL_POSITION),
            RenderLayers::layer(0).with(1),
            Name::new("Web cosmic fill light"),
        ));
    } else {
        // Native renderers can use a dedicated shadowless key in the
        // reflection pass without spending another set of shadow maps.
        commands.spawn((
            DirectionalLight {
                color: NEBULA_KEY_COLOR,
                illuminance: NEBULA_KEY_ILLUMINANCE,
                shadow_maps_enabled: false,
                ..default()
            },
            key_light_transform,
            RenderLayers::layer(1),
            Name::new("Reflected nebula key light"),
        ));

        // One shadowless fill serves both the main and reflection cameras. Its
        // opposite azimuth prevents the pink key from flattening silhouettes.
        commands.spawn((
            DirectionalLight {
                color: COSMIC_FILL_COLOR,
                illuminance: COSMIC_FILL_ILLUMINANCE,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.6, 2.35, 0.18)),
            RenderLayers::layer(0).with(1),
            Name::new("Cosmic fill light"),
        ));
    }

    let mut bloom = Bloom::NATURAL;
    bloom.intensity = BLOOM_INTENSITY;
    bloom.low_frequency_boost = BLOOM_LOW_FREQUENCY_BOOST;
    bloom.high_pass_frequency = BLOOM_HIGH_PASS_FREQUENCY;
    bloom.prefilter = BloomPrefilter {
        threshold: BLOOM_THRESHOLD,
        threshold_softness: BLOOM_THRESHOLD_SOFTNESS,
    };
    bloom.composite_mode = BloomCompositeMode::Additive;
    bloom.max_mip_dimension = BLOOM_MAX_MIP_DIMENSION;

    let mut camera = commands.spawn((
        Camera3d::default(),
        Hdr,
        Msaa::Off,
        Smaa {
            preset: SmaaPreset::Ultra,
        },
        MipBias(MATERIAL_TEXTURE_MIP_BIAS),
        Tonemapping::TonyMcMapface,
        bloom,
        ColorGrading {
            global: ColorGradingGlobal {
                exposure: COLOR_GRADING_EXPOSURE,
                tint: COLOR_GRADING_TINT,
                post_saturation: COLOR_GRADING_SATURATION,
                ..default()
            },
            ..default()
        },
        Vignette {
            intensity: VIGNETTE_INTENSITY,
            radius: VIGNETTE_RADIUS,
            smoothness: VIGNETTE_SMOOTHNESS,
            ..default()
        },
        Transform::from_xyz(0.0, 10.5, -13.5).looking_at(Vec3::ZERO, Vec3::Y),
        chess_camera_controller(),
    ));
    camera.with_children(|camera| {
        if cfg!(target_arch = "wasm32") {
            camera.spawn((
                PointLight {
                    color: CAMERA_FILL_COLOR,
                    intensity: point_light_lumens(CAMERA_FILL_ILLUMINANCE, HOME_CAMERA_RADIUS),
                    range: web_camera_fill_range(HOME_CAMERA_RADIUS),
                    radius: WEB_CAMERA_FILL_RADIUS,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::IDENTITY,
                RenderLayers::layer(0),
                WebCameraReadabilityFill,
                Name::new("Web camera readability fill"),
            ));
        } else {
            camera.spawn((
                DirectionalLight {
                    color: CAMERA_FILL_COLOR,
                    illuminance: CAMERA_FILL_ILLUMINANCE,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_rotation(Quat::from_euler(
                    EulerRot::XYZ,
                    CAMERA_FILL_PITCH_DEGREES.to_radians(),
                    CAMERA_FILL_YAW_DEGREES.to_radians(),
                    0.0,
                )),
                RenderLayers::layer(0),
                Name::new("Camera readability fill"),
            ));
        }
    });
}

fn point_light_lumens(illuminance: f32, distance: f32) -> f32 {
    illuminance * 4.0 * PI * distance * distance
}

fn web_camera_fill_range(camera_radius: f32) -> f32 {
    (camera_radius * WEB_CAMERA_FILL_RANGE_MULTIPLIER).max(WEB_CAMERA_FILL_MIN_RANGE)
}

#[cfg(target_arch = "wasm32")]
fn update_web_camera_fill(
    camera: Single<&PanOrbitCamera>,
    mut fill: Single<&mut PointLight, With<WebCameraReadabilityFill>>,
) {
    let radius = camera.radius.unwrap_or(HOME_CAMERA_RADIUS).max(1.0);
    let target_intensity = point_light_lumens(CAMERA_FILL_ILLUMINANCE, radius);
    let target_range = web_camera_fill_range(radius);

    if (fill.intensity - target_intensity).abs() > 1.0 {
        fill.intensity = target_intensity;
    }
    if (fill.range - target_range).abs() > 0.01 {
        fill.range = target_range;
    }
}

fn chess_camera_controller() -> PanOrbitCamera {
    PanOrbitCamera {
        button_orbit: MouseButton::Left,
        button_pan: MouseButton::Right,
        zoom_lower_limit: 6.0,
        zoom_upper_limit: Some(25.0),
        orbit_sensitivity: MANUAL_ORBIT_SENSITIVITY,
        orbit_smoothness: MANUAL_ORBIT_SMOOTHNESS,
        // One-finger drag orbits. With two fingers, midpoint motion pans and
        // changing the distance between the touches performs pinch zoom.
        touch_enabled: true,
        touch_controls: TouchControls::OneFingerOrbit,
        ..default()
    }
}

fn orient_camera_after_local_move(
    chess_match: Res<ChessMatch>,
    menu: Res<GameMenuState>,
    animation: Res<PieceAnimationState>,
    mut state: ResMut<LocalCameraState>,
    mut auto_turn: ResMut<CameraAutoTurn>,
    mut camera: Single<&mut PanOrbitCamera>,
) {
    if state.observed_generation == chess_match.generation {
        return;
    }
    let action = generation_camera_action(
        menu.open,
        menu.active_mode,
        chess_match.last_move.is_some(),
        animation.is_settled(chess_match.generation),
        chess_match.game.position().side_to_move(),
    );
    match action {
        GenerationCameraAction::Wait => return,
        GenerationCameraAction::Observe => {}
        GenerationCameraAction::Orient(side) => {
            start_camera_turn(&mut camera, &mut auto_turn, side);
        }
    }
    state.observed_generation = chess_match.generation;
}

fn generation_camera_action(
    menu_open: bool,
    mode: GameMode,
    has_last_move: bool,
    animation_settled: bool,
    side_to_move: Side,
) -> GenerationCameraAction {
    if menu_open {
        return GenerationCameraAction::Observe;
    }
    if !animation_settled {
        return GenerationCameraAction::Wait;
    }
    if mode == GameMode::Local && has_last_move {
        GenerationCameraAction::Orient(side_to_move)
    } else {
        // New games keep the orientation selected by the start button. AI
        // games likewise stay on the human player's side after every move.
        GenerationCameraAction::Observe
    }
}

pub(crate) fn start_camera_turn(
    camera: &mut PanOrbitCamera,
    auto_turn: &mut CameraAutoTurn,
    side: Side,
) {
    start_camera_transition(camera, auto_turn, side, None);
}

fn start_camera_recenter(
    camera: &mut PanOrbitCamera,
    auto_turn: &mut CameraAutoTurn,
    side: Side,
    radius: f32,
) {
    start_camera_transition(
        camera,
        auto_turn,
        side,
        Some((HOME_CAMERA_PITCH_RADIANS, radius)),
    );
}

fn start_camera_transition(
    camera: &mut PanOrbitCamera,
    auto_turn: &mut CameraAutoTurn,
    side: Side,
    full_target: Option<(f32, f32)>,
) {
    let base_yaw = match side {
        Side::White => PI,
        Side::Black => 0.0,
    };
    let current_yaw = camera.yaw.unwrap_or(camera.target_yaw);
    let current_focus = camera.focus;
    let full_recenter = full_target.map(|(end_pitch, end_radius)| FullCameraRecenter {
        start_pitch: camera.pitch.unwrap_or(camera.target_pitch),
        end_pitch,
        start_radius: camera.radius.unwrap_or(camera.target_radius),
        end_radius,
    });

    // Pan-orbit yaw is unbounded. Pick the equivalent side angle closest to
    // the current camera so opening a game never causes a needless full turn.
    let end_yaw = nearest_equivalent_angle(base_yaw, current_yaw);
    let already_fully_centered = full_recenter.as_ref().is_none_or(|recenter| {
        (recenter.end_pitch - recenter.start_pitch).abs() < 0.001
            && (recenter.end_radius - recenter.start_radius).abs() < 0.001
    });
    if (end_yaw - current_yaw).abs() < 0.001
        && current_focus.length_squared() < 0.000_001
        && already_fully_centered
    {
        camera.yaw = Some(end_yaw);
        camera.target_yaw = end_yaw;
        camera.focus = Vec3::ZERO;
        camera.target_focus = Vec3::ZERO;
        if let Some(recenter) = full_recenter {
            camera.pitch = Some(recenter.end_pitch);
            camera.target_pitch = recenter.end_pitch;
            camera.radius = Some(recenter.end_radius);
            camera.target_radius = recenter.end_radius;
        }
        camera.enabled = true;
        camera.force_update = true;
        auto_turn.active = None;
        return;
    }

    camera.target_yaw = current_yaw;
    camera.target_focus = current_focus;
    if let Some(recenter) = &full_recenter {
        camera.target_pitch = recenter.start_pitch;
        camera.target_radius = recenter.start_radius;
    }
    camera.enabled = false;
    camera.force_update = true;
    auto_turn.active = Some(AutomaticCameraTurn {
        start_yaw: current_yaw,
        end_yaw,
        start_focus: current_focus,
        full_recenter,
        elapsed: 0.0,
    });
}

fn handle_manual_camera_recenter(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    menu: Res<GameMenuState>,
    chess_match: Res<ChessMatch>,
    mut auto_turn: ResMut<CameraAutoTurn>,
    mut camera: Single<&mut PanOrbitCamera>,
) {
    if menu.open || !mouse_buttons.just_pressed(MouseButton::Middle) {
        return;
    }
    let side = manual_recenter_side(
        menu.active_mode,
        menu.active_side,
        chess_match.game.position().side_to_move(),
    );
    let radius = board_camera_radius(chess_match.game.position().board().size());
    start_camera_recenter(&mut camera, &mut auto_turn, side, radius);
}

fn manual_recenter_side(active_mode: GameMode, player_side: Side, side_to_move: Side) -> Side {
    match active_mode {
        GameMode::Local => side_to_move,
        GameMode::Ai | GameMode::Multiplayer => player_side,
    }
}

fn animate_automatic_camera_turn(
    time: Res<Time>,
    menu: Res<GameMenuState>,
    mut auto_turn: ResMut<CameraAutoTurn>,
    mut camera: Single<&mut PanOrbitCamera>,
) {
    if menu.open {
        auto_turn.active = None;
        return;
    }
    let Some(turn) = auto_turn.active.as_mut() else {
        return;
    };

    turn.elapsed += time.delta_secs();
    let turn_progress = (turn.elapsed / AUTO_TURN_SECONDS).min(1.0);
    let recenter_progress = (turn.elapsed / AUTO_RECENTER_SECONDS).min(1.0);
    let yaw = turn
        .start_yaw
        .lerp(turn.end_yaw, smootherstep(turn_progress));
    let focus = turn
        .start_focus
        .lerp(Vec3::ZERO, smootherstep(recenter_progress));
    camera.yaw = Some(yaw);
    camera.target_yaw = yaw;
    camera.focus = focus;
    camera.target_focus = focus;
    if let Some(recenter) = &turn.full_recenter {
        let pitch = recenter
            .start_pitch
            .lerp(recenter.end_pitch, smootherstep(turn_progress));
        let radius = recenter
            .start_radius
            .lerp(recenter.end_radius, smootherstep(turn_progress));
        camera.pitch = Some(pitch);
        camera.target_pitch = pitch;
        camera.radius = Some(radius);
        camera.target_radius = radius;
    }
    camera.enabled = turn_progress >= 1.0 && recenter_progress >= 1.0;
    camera.force_update = true;

    if camera.enabled {
        auto_turn.active = None;
    }
}

fn smootherstep(value: f32) -> f32 {
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}

fn nearest_equivalent_angle(angle: f32, reference: f32) -> f32 {
    angle + ((reference - angle) / TAU).round() * TAU
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_camera_uses_pinch_zoom_and_two_finger_pan_scheme() {
        let camera = chess_camera_controller();
        assert!(camera.touch_enabled);
        assert_eq!(camera.touch_controls, TouchControls::OneFingerOrbit);
        assert_eq!(camera.orbit_sensitivity, MANUAL_ORBIT_SENSITIVITY);
        assert_eq!(camera.orbit_smoothness, MANUAL_ORBIT_SMOOTHNESS);
    }

    #[test]
    fn web_point_fill_preserves_illuminance_across_camera_zoom() {
        let near = point_light_lumens(CAMERA_FILL_ILLUMINANCE, 6.0);
        let far = point_light_lumens(CAMERA_FILL_ILLUMINANCE, 12.0);
        assert!((far / near - 4.0).abs() < 0.001);
        assert_eq!(web_camera_fill_range(6.0), 24.0);
        assert_eq!(web_camera_fill_range(20.0), 40.0);
    }

    #[test]
    fn side_orientation_uses_the_nearest_equivalent_turn() {
        assert!((nearest_equivalent_angle(0.0, TAU - 0.1) - TAU).abs() < 0.001);
        assert!((nearest_equivalent_angle(PI, -3.0) + PI).abs() < 0.001);
    }

    #[test]
    fn automatic_turn_easing_has_smooth_exact_endpoints() {
        assert_eq!(smootherstep(0.0), 0.0);
        assert_eq!(smootherstep(1.0), 1.0);
        assert!((smootherstep(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn a_new_game_keeps_its_explicitly_selected_orientation() {
        assert_eq!(
            generation_camera_action(false, GameMode::Local, false, true, Side::White),
            GenerationCameraAction::Observe
        );
    }

    #[test]
    fn a_local_turn_waits_for_piece_animation_before_rotating() {
        assert_eq!(
            generation_camera_action(false, GameMode::Local, true, false, Side::Black),
            GenerationCameraAction::Wait
        );
        assert_eq!(
            generation_camera_action(false, GameMode::Local, true, true, Side::Black),
            GenerationCameraAction::Orient(Side::Black)
        );
    }

    #[test]
    fn an_ai_game_keeps_the_human_camera_orientation() {
        assert_eq!(
            generation_camera_action(false, GameMode::Ai, true, true, Side::Black),
            GenerationCameraAction::Observe
        );
    }

    #[test]
    fn manual_recenter_targets_the_turn_in_local_and_the_human_in_ai() {
        assert_eq!(
            manual_recenter_side(GameMode::Local, Side::White, Side::Black),
            Side::Black
        );
        assert_eq!(
            manual_recenter_side(GameMode::Ai, Side::White, Side::Black),
            Side::White
        );
    }

    #[test]
    fn full_recenter_captures_pitch_radius_and_focus_for_animation() {
        let mut camera = PanOrbitCamera {
            focus: Vec3::new(1.0, 2.0, 3.0),
            target_focus: Vec3::new(4.0, 5.0, 6.0),
            yaw: Some(PI / 2.0),
            target_yaw: PI / 2.0,
            pitch: Some(0.2),
            target_pitch: 0.3,
            radius: Some(9.0),
            target_radius: 10.0,
            ..default()
        };
        let mut auto_turn = CameraAutoTurn::default();

        start_camera_recenter(&mut camera, &mut auto_turn, Side::White, 14.2);

        let transition = auto_turn.active.expect("recenter should animate");
        let full = transition
            .full_recenter
            .expect("manual recenter affects every orbit axis");
        assert_eq!(transition.start_focus, Vec3::new(1.0, 2.0, 3.0));
        assert!((transition.end_yaw - PI).abs() < 0.001);
        assert!((full.start_pitch - 0.2).abs() < 0.001);
        assert!((full.end_pitch - HOME_CAMERA_PITCH_RADIANS).abs() < 0.001);
        assert!((full.start_radius - 9.0).abs() < 0.001);
        assert!((full.end_radius - 14.2).abs() < 0.001);
        assert!(!camera.enabled);
    }
}
