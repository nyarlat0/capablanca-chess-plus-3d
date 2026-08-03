use std::f32::consts::PI;

use bevy::{
    camera::visibility::RenderLayers, gltf::GltfAssetLabel, prelude::*,
    world_serialization::WorldInstanceReady,
};
use capablanca_chess_plus::{BoardSize, Color as Side, MoveKind, Piece, PieceKind, Square};

use crate::{
    app::FrontendSet,
    board::{BOARD_BASE_BOTTOM_Y, SQUARE_SIZE, board_world_size, square_world},
    game::{ChessMatch, piece_name, side_name},
    reflection::ReflectedPieceMesh,
    render_tuning::{
        BLACK_PIECE_CLEARCOAT, BLACK_PIECE_CLEARCOAT_ROUGHNESS, BLACK_PIECE_COLOR,
        BLACK_PIECE_REFLECTANCE, BLACK_PIECE_ROUGHNESS,
    },
};

const MOVE_ANIMATION_SECONDS: f32 = 0.38;
const MOVE_ARC_HEIGHT: f32 = 0.9;
const CAPTURE_ANIMATION_SECONDS: f32 = 0.72;
const CAPTURE_ARC_HEIGHT: f32 = 1.25;
const CAPTURED_PIECE_SCALE: f32 = 0.72;
const CAPTURE_TRAY_EDGE_GAP: f32 = 0.85;
const CAPTURE_TRAY_COLUMN_GAP: f32 = 0.72;
// User-facing size multiplier for every piece. 1.0 is the original GLB size
// after unit conversion; the current 1.3 makes every piece 30% larger.
const PIECE_SCALE: f32 = 1.3;
const GLB_TO_BOARD_SCALE: f32 = 0.018;

pub(crate) struct PiecesPlugin;

impl Plugin for PiecesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PieceAnimationState>()
            .add_observer(apply_piece_material)
            .add_systems(Startup, setup_piece_assets)
            .add_systems(Update, sync_pieces.in_set(FrontendSet::PieceSync))
            .add_systems(Update, animate_pieces.in_set(FrontendSet::Animation));
    }
}

#[derive(Resource)]
pub(crate) struct PieceAssets {
    pawn: Handle<WorldAsset>,
    knight: Handle<WorldAsset>,
    bishop: Handle<WorldAsset>,
    rook: Handle<WorldAsset>,
    queen: Handle<WorldAsset>,
    king: Handle<WorldAsset>,
    archbishop: Handle<WorldAsset>,
    chancellor: Handle<WorldAsset>,
    white_material: Handle<StandardMaterial>,
    black_material: Handle<StandardMaterial>,
}

impl PieceAssets {
    pub(crate) fn scene(&self, kind: PieceKind) -> Handle<WorldAsset> {
        match kind {
            PieceKind::Pawn => &self.pawn,
            PieceKind::Knight => &self.knight,
            PieceKind::Bishop => &self.bishop,
            PieceKind::Rook => &self.rook,
            PieceKind::Queen => &self.queen,
            PieceKind::King => &self.king,
            PieceKind::Archbishop => &self.archbishop,
            PieceKind::Chancellor => &self.chancellor,
        }
        .clone()
    }

    pub(crate) fn material(&self, side: Side) -> Handle<StandardMaterial> {
        match side {
            Side::White => self.white_material.clone(),
            Side::Black => self.black_material.clone(),
        }
    }
}

#[derive(Resource)]
pub(crate) struct PieceAnimationState {
    rendered_generation: u64,
    active_motions: usize,
    landed_generation: Option<u64>,
}

impl Default for PieceAnimationState {
    fn default() -> Self {
        Self {
            rendered_generation: u64::MAX,
            active_motions: 0,
            landed_generation: None,
        }
    }
}

impl PieceAnimationState {
    pub(crate) fn is_settled(&self, generation: u64) -> bool {
        self.rendered_generation == generation && self.active_motions == 0
    }

    fn begin_generation(&mut self, generation: u64, active_motions: usize) {
        self.rendered_generation = generation;
        self.active_motions = active_motions;
        self.landed_generation = None;
    }

    fn finish_motion(&mut self, generation: u64) {
        if self.rendered_generation == generation {
            self.active_motions = self.active_motions.saturating_sub(1);
        }
    }

    pub(crate) fn has_move_landed(&self, generation: u64) -> bool {
        self.landed_generation == Some(generation)
    }

    fn mark_move_landed(&mut self, generation: u64) {
        if self.rendered_generation == generation {
            self.landed_generation = Some(generation);
        }
    }
}

#[derive(Component)]
pub(crate) struct PieceRoot;

#[derive(Component)]
pub(crate) struct PieceMaterial {
    material: Handle<StandardMaterial>,
    layers: RenderLayers,
    reflected: bool,
}

impl PieceMaterial {
    fn board(material: Handle<StandardMaterial>) -> Self {
        Self {
            material,
            layers: RenderLayers::layer(0).with(1),
            reflected: true,
        }
    }

    pub(crate) fn preview(material: Handle<StandardMaterial>, layer: usize) -> Self {
        Self {
            material,
            layers: RenderLayers::layer(layer),
            reflected: false,
        }
    }
}

#[derive(Component)]
struct PieceMotion {
    start: Vec3,
    end: Vec3,
    start_scale: Vec3,
    end_scale: Vec3,
    duration: f32,
    arc_height: f32,
    elapsed: f32,
    generation: u64,
    signals_move_landing: bool,
}

pub(crate) const fn piece_model_scale() -> f32 {
    GLB_TO_BOARD_SCALE * PIECE_SCALE
}

fn setup_piece_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(PieceAssets {
        pawn: load_piece_scene(&asset_server, "models/pawn.glb"),
        knight: load_piece_scene(&asset_server, "models/knight.glb"),
        bishop: load_piece_scene(&asset_server, "models/bishop.glb"),
        rook: load_piece_scene(&asset_server, "models/rook.glb"),
        queen: load_piece_scene(&asset_server, "models/queen.glb"),
        king: load_piece_scene(&asset_server, "models/king.glb"),
        archbishop: load_piece_scene(&asset_server, "models/archbishop.glb"),
        chancellor: load_piece_scene(&asset_server, "models/chancellor.glb"),
        white_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.92, 0.82, 0.66),
            perceptual_roughness: 0.36,
            reflectance: 0.4,
            clearcoat: 0.08,
            clearcoat_perceptual_roughness: 0.25,
            ..default()
        }),
        black_material: materials.add(StandardMaterial {
            base_color: BLACK_PIECE_COLOR,
            metallic: 0.0,
            perceptual_roughness: BLACK_PIECE_ROUGHNESS,
            reflectance: BLACK_PIECE_REFLECTANCE,
            clearcoat: BLACK_PIECE_CLEARCOAT,
            clearcoat_perceptual_roughness: BLACK_PIECE_CLEARCOAT_ROUGHNESS,
            ..default()
        }),
    });
}

fn load_piece_scene(asset_server: &AssetServer, path: &'static str) -> Handle<WorldAsset> {
    asset_server.load(GltfAssetLabel::Scene(0).from_asset(path))
}

fn sync_pieces(
    mut commands: Commands,
    chess_match: Res<ChessMatch>,
    assets: Res<PieceAssets>,
    mut animation: ResMut<PieceAnimationState>,
    pieces: Query<Entity, With<PieceRoot>>,
) {
    if animation.rendered_generation == chess_match.generation {
        return;
    }

    let position = chess_match.game.position();
    let size = position.board().size();
    for entity in &pieces {
        commands.entity(entity).despawn();
    }
    let mut active_motions = 0;
    for (square, piece) in position.board().pieces() {
        let target = square_world(square, size);
        let start = animation_start(&chess_match, square, piece)
            .map_or(target, |source| square_world(source, size));
        active_motions += usize::from(spawn_piece(
            &mut commands,
            &assets,
            square,
            piece,
            start,
            target,
            chess_match.generation,
            chess_match
                .last_move
                .is_some_and(|chess_move| square == chess_move.to),
        ));
    }
    for captured_by in Side::ALL {
        for captured in chess_match
            .captured_pieces
            .iter()
            .filter(|captured| captured.captured_by == captured_by)
        {
            active_motions += usize::from(spawn_captured_piece(
                &mut commands,
                &assets,
                captured,
                captured.tray_slot,
                size,
                chess_match.generation,
            ));
        }
    }
    animation.begin_generation(chess_match.generation, active_motions);
}

fn animation_start(chess_match: &ChessMatch, square: Square, piece: Piece) -> Option<Square> {
    if !chess_match.animate_last_move {
        return None;
    }
    let chess_move = chess_match.last_move?;
    if square == chess_move.to {
        return Some(chess_move.from);
    }
    let MoveKind::Castle(side) = chess_move.kind else {
        return None;
    };
    let route = chess_match
        .game
        .position()
        .rules()
        .castling()
        .route(piece.color, side)?;
    (square == route.rook_to).then_some(route.rook_from)
}

fn spawn_piece(
    commands: &mut Commands,
    assets: &PieceAssets,
    square: Square,
    piece: Piece,
    start: Vec3,
    target: Vec3,
    generation: u64,
    signals_move_landing: bool,
) -> bool {
    let scale = Vec3::splat(piece_model_scale());
    let mut entity = commands.spawn((
        WorldAssetRoot(assets.scene(piece.kind)),
        Transform::from_translation(start)
            .with_rotation(Quat::from_rotation_y(if piece.color == Side::Black {
                PI
            } else {
                0.0
            }))
            .with_scale(scale),
        PieceRoot,
        PieceMaterial::board(assets.material(piece.color)),
        Name::new(format!(
            "{} {} on {}",
            side_name(piece.color),
            piece_name(piece.kind),
            square
        )),
    ));
    let animated = start != target;
    if animated {
        entity.insert(PieceMotion {
            start,
            end: target,
            start_scale: scale,
            end_scale: scale,
            duration: MOVE_ANIMATION_SECONDS,
            arc_height: MOVE_ARC_HEIGHT,
            elapsed: 0.0,
            generation,
            signals_move_landing,
        });
    }
    animated
}

fn spawn_captured_piece(
    commands: &mut Commands,
    assets: &PieceAssets,
    captured: &crate::game::CapturedPiece,
    slot: usize,
    size: BoardSize,
    generation: u64,
) -> bool {
    let target = captured_piece_world(captured.captured_by, slot, size);
    let is_new_capture = captured.generation == generation;
    let start = if is_new_capture {
        square_world(captured.from, size)
    } else {
        target
    };
    let full_scale = Vec3::splat(piece_model_scale());
    let tray_scale = full_scale * CAPTURED_PIECE_SCALE;
    let start_scale = if is_new_capture {
        full_scale
    } else {
        tray_scale
    };
    let piece = captured.piece;
    let mut entity = commands.spawn((
        WorldAssetRoot(assets.scene(piece.kind)),
        Transform::from_translation(start)
            .with_rotation(Quat::from_rotation_y(if piece.color == Side::Black {
                PI
            } else {
                0.0
            }))
            .with_scale(start_scale),
        PieceRoot,
        PieceMaterial::board(assets.material(piece.color)),
        Name::new(format!(
            "Captured {} {} #{}",
            side_name(piece.color),
            piece_name(piece.kind),
            captured.id
        )),
    ));
    if is_new_capture {
        entity.insert(PieceMotion {
            start,
            end: target,
            start_scale,
            end_scale: tray_scale,
            duration: CAPTURE_ANIMATION_SECONDS,
            arc_height: CAPTURE_ARC_HEIGHT,
            elapsed: 0.0,
            generation,
            signals_move_landing: false,
        });
    }
    is_new_capture
}

fn captured_piece_world(captured_by: Side, slot: usize, size: BoardSize) -> Vec3 {
    let rows = usize::from(size.ranks());
    let row = slot % rows;
    let column = slot / rows;
    let board_size = board_world_size(size);
    let width = board_size.x;
    let depth = board_size.y;
    let x_offset = width * 0.5 + CAPTURE_TRAY_EDGE_GAP + column as f32 * CAPTURE_TRAY_COLUMN_GAP;
    let z_from_white_side = row as f32 * SQUARE_SIZE - (depth - SQUARE_SIZE) * 0.5;
    match captured_by {
        // A camera facing the board from White's side has world -X on its
        // right; from Black's side that direction is world +X.
        Side::White => Vec3::new(-x_offset, BOARD_BASE_BOTTOM_Y, z_from_white_side),
        Side::Black => Vec3::new(x_offset, BOARD_BASE_BOTTOM_Y, -z_from_white_side),
    }
}

fn apply_piece_material(
    scene_ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    piece_materials: Query<&PieceMaterial>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let Ok(presentation) = piece_materials.get(scene_ready.entity) else {
        return;
    };

    for descendant in children.iter_descendants(scene_ready.entity) {
        if meshes.contains(descendant) {
            let mut entity = commands.entity(descendant);
            entity.insert((
                MeshMaterial3d(presentation.material.clone()),
                presentation.layers.clone(),
                Pickable::IGNORE,
            ));
            if presentation.reflected {
                entity.insert(ReflectedPieceMesh);
            } else {
                entity.remove::<ReflectedPieceMesh>();
            }
        }
    }
}

fn animate_pieces(
    mut commands: Commands,
    time: Res<Time>,
    mut animation: ResMut<PieceAnimationState>,
    mut pieces: Query<(Entity, &mut Transform, &mut PieceMotion)>,
) {
    for (entity, mut transform, mut motion) in &mut pieces {
        motion.elapsed += time.delta_secs();
        let t = (motion.elapsed / motion.duration).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        transform.translation = motion.start.lerp(motion.end, eased);
        transform.translation.y += motion.arc_height * 4.0 * t * (1.0 - t);
        transform.scale = motion.start_scale.lerp(motion.end_scale, eased);
        if t >= 1.0 {
            transform.translation = motion.end;
            transform.scale = motion.end_scale;
            if motion.signals_move_landing {
                animation.mark_move_landed(motion.generation);
            }
            animation.finish_motion(motion.generation);
            commands.entity(entity).remove::<PieceMotion>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_settles_only_after_every_current_motion_finishes() {
        let mut state = PieceAnimationState::default();
        state.begin_generation(7, 2);
        assert!(!state.is_settled(7));

        state.finish_motion(6);
        state.finish_motion(7);
        assert!(!state.is_settled(7));

        state.finish_motion(7);
        assert!(state.is_settled(7));
    }

    #[test]
    fn captured_pieces_are_placed_on_the_capturers_right() {
        let size = BoardSize::CAPABLANCA;
        let white_capture = captured_piece_world(Side::White, 0, size);
        let black_capture = captured_piece_world(Side::Black, 0, size);
        let board_size = board_world_size(size);

        assert!(white_capture.x < -board_size.x * 0.5);
        assert!(black_capture.x > board_size.x * 0.5);
        assert_eq!(white_capture.y, BOARD_BASE_BOTTOM_Y);
        assert_eq!(black_capture.y, BOARD_BASE_BOTTOM_Y);
        assert_eq!(white_capture.z, -black_capture.z);
    }
}
