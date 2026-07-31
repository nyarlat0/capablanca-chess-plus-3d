use std::f32::consts::PI;

use bevy::{gltf::GltfAssetLabel, prelude::*, world_serialization::WorldInstanceReady};
use capablanca_chess_plus::{Color as Side, MoveKind, Piece, PieceKind, Square};

use crate::{
    app::FrontendSet,
    board::square_world,
    game::{ChessMatch, piece_name, side_name},
};

const MOVE_ANIMATION_SECONDS: f32 = 0.38;
const PIECE_MODEL_SCALE: f32 = 0.018;

pub(crate) struct PiecesPlugin;

impl Plugin for PiecesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PieceRenderState>()
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
struct PieceRenderState {
    generation: u64,
}

impl Default for PieceRenderState {
    fn default() -> Self {
        Self {
            generation: u64::MAX,
        }
    }
}

#[derive(Component)]
struct PieceRoot;

#[derive(Component)]
struct PieceMaterial(Handle<StandardMaterial>);

#[derive(Component)]
struct PieceMotion {
    start: Vec3,
    end: Vec3,
    elapsed: f32,
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
    mut rendered: ResMut<PieceRenderState>,
    pieces: Query<Entity, With<PieceRoot>>,
) {
    if rendered.generation == chess_match.generation {
        return;
    }

    let position = chess_match.game.position();
    let size = position.board().size();
    for entity in &pieces {
        commands.entity(entity).despawn();
    }
    for (square, piece) in position.board().pieces() {
        let target = square_world(square, size);
        let start = animation_start(&chess_match, square, piece)
            .map_or(target, |source| square_world(source, size));
        spawn_piece(&mut commands, &assets, square, piece, start, target);
    }
    rendered.generation = chess_match.generation;
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
) {
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
    if start != target {
        entity.insert(PieceMotion {
            start,
            end: target,
            elapsed: 0.0,
        });
    }
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
            commands
                .entity(descendant)
                .insert((MeshMaterial3d(material.0.clone()), Pickable::IGNORE));
        }
    }
}

fn animate_pieces(
    mut commands: Commands,
    time: Res<Time>,
    mut pieces: Query<(Entity, &mut Transform, &mut PieceMotion)>,
) {
    for (entity, mut transform, mut motion) in &mut pieces {
        motion.elapsed += time.delta_secs();
        let t = (motion.elapsed / MOVE_ANIMATION_SECONDS).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        transform.translation = motion.start.lerp(motion.end, eased);
        transform.translation.y += 0.9 * 4.0 * t * (1.0 - t);
        if t >= 1.0 {
            transform.translation = motion.end;
            commands.entity(entity).remove::<PieceMotion>();
        }
    }
}
