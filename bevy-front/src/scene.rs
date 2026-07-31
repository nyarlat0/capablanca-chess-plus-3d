use std::f32::consts::{PI, TAU};

use bevy::{camera::visibility::RenderLayers, prelude::*};
use bevy_panorbit_camera::PanOrbitCamera;
use capablanca_chess_plus::Color as Side;

use crate::{
    app::FrontendSet,
    game::ChessMatch,
    menu::{GameMenuState, GameMode},
    pieces::PieceAnimationState,
};

// Duration of the automatic half-turn after a move or starting a game.
// This does not affect mouse-controlled camera movement.
const AUTO_TURN_SECONDS: f32 = 1.4;
// Recentring should finish sooner than the turn, but still ease in and out.
const AUTO_RECENTER_SECONDS: f32 = 0.5;

pub(crate) struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalCameraState>()
            .init_resource::<CameraAutoTurn>()
            .add_systems(Startup, setup_environment)
            .add_systems(
                Update,
                (
                    orient_camera_after_local_move,
                    animate_automatic_camera_turn,
                )
                    .chain()
                    .in_set(FrontendSet::Camera),
            );
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
    elapsed: f32,
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
    let base_yaw = match side {
        Side::White => PI,
        Side::Black => 0.0,
    };
    let current_yaw = camera.yaw.unwrap_or(camera.target_yaw);
    let current_focus = camera.focus;

    // Pan-orbit yaw is unbounded. Pick the equivalent side angle closest to
    // the current camera so opening a game never causes a needless full turn.
    let end_yaw = nearest_equivalent_angle(base_yaw, current_yaw);
    if (end_yaw - current_yaw).abs() < 0.001 && current_focus.length_squared() < 0.000_001 {
        camera.yaw = Some(end_yaw);
        camera.target_yaw = end_yaw;
        camera.focus = Vec3::ZERO;
        camera.target_focus = Vec3::ZERO;
        camera.enabled = true;
        camera.force_update = true;
        auto_turn.active = None;
        return;
    }

    camera.target_yaw = current_yaw;
    camera.target_focus = current_focus;
    camera.enabled = false;
    camera.force_update = true;
    auto_turn.active = Some(AutomaticCameraTurn {
        start_yaw: current_yaw,
        end_yaw,
        start_focus: current_focus,
        elapsed: 0.0,
    });
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
}
