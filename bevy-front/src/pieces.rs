use std::f32::consts::PI;

use bevy::{
    camera::visibility::RenderLayers, gltf::GltfAssetLabel, prelude::*,
    world_serialization::WorldInstanceReady,
};
use capablanca_chess_plus::{Color as Side, MoveKind, Piece, PieceKind, Square};

use crate::{
    app::FrontendSet,
    board::square_world,
    game::{ChessMatch, piece_name, side_name},
    reflection::ReflectedPieceMesh,
};

const MOVE_ANIMATION_SECONDS: f32 = 0.38;
const MOVE_ARC_HEIGHT: f32 = 0.9;
const PIECE_MODEL_SCALE: f32 = 0.018;

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
struct PieceAssets {
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

#[derive(Resource)]
pub(crate) struct PieceAnimationState {
    rendered_generation: u64,
    active_motions: usize,
}

impl Default for PieceAnimationState {
    fn default() -> Self {
        Self {
            rendered_generation: u64::MAX,
            active_motions: 0,
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
    }

    fn finish_motion(&mut self, generation: u64) {
        if self.rendered_generation == generation {
            self.active_motions = self.active_motions.saturating_sub(1);
        }
    }
}

#[derive(Component)]
pub(crate) struct PieceRoot;

#[derive(Component)]
struct PieceMaterial(Handle<StandardMaterial>);

#[derive(Component)]
struct PieceMotion {
    start: Vec3,
    end: Vec3,
    elapsed: f32,
    generation: u64,
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
            base_color: Color::srgb(0.075, 0.055, 0.05),
            perceptual_roughness: 0.3,
            reflectance: 0.4,
            clearcoat: 0.08,
            clearcoat_perceptual_roughness: 0.25,
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
        ));
    }
    animation.begin_generation(chess_match.generation, active_motions);
}

fn animation_start(chess_match: &ChessMatch, square: Square, piece: Piece) -> Option<Square> {
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
) -> bool {
    let material = match piece.color {
        Side::White => assets.white_material.clone(),
        Side::Black => assets.black_material.clone(),
    };
    let mut entity = commands.spawn((
        WorldAssetRoot(piece_scene(piece.kind, assets)),
        Transform::from_translation(start)
            .with_rotation(Quat::from_rotation_y(if piece.color == Side::Black {
                PI
            } else {
                0.0
            }))
            .with_scale(Vec3::splat(PIECE_MODEL_SCALE)),
        PieceRoot,
        PieceMaterial(material),
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
            elapsed: 0.0,
            generation,
        });
    }
    animated
}

fn piece_scene(kind: PieceKind, assets: &PieceAssets) -> Handle<WorldAsset> {
    match kind {
        PieceKind::Pawn => &assets.pawn,
        PieceKind::Knight => &assets.knight,
        PieceKind::Bishop => &assets.bishop,
        PieceKind::Rook => &assets.rook,
        PieceKind::Queen => &assets.queen,
        PieceKind::King => &assets.king,
        PieceKind::Archbishop => &assets.archbishop,
        PieceKind::Chancellor => &assets.chancellor,
    }
    .clone()
}

fn apply_piece_material(
    scene_ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    piece_materials: Query<&PieceMaterial>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let Ok(material) = piece_materials.get(scene_ready.entity) else {
        return;
    };

    for descendant in children.iter_descendants(scene_ready.entity) {
        if meshes.contains(descendant) {
            commands.entity(descendant).insert((
                MeshMaterial3d(material.0.clone()),
                RenderLayers::layer(0).with(1),
                ReflectedPieceMesh,
                Pickable::IGNORE,
            ));
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
        let t = (motion.elapsed / MOVE_ANIMATION_SECONDS).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        transform.translation = motion.start.lerp(motion.end, eased);
        transform.translation.y += MOVE_ARC_HEIGHT * 4.0 * t * (1.0 - t);
        if t >= 1.0 {
            transform.translation = motion.end;
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
}
