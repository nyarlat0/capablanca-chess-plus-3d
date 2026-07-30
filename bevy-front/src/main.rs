use bevy::{
    color::LinearRgba,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures_lite::future},
};
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};
use capablanca_chess_plus::{
    BoardSize, CastleSide, Color as Side, DrawReason, Engine, Game, GameOutcome, Move, MoveKind,
    Piece, PieceKind, SearchLimits, SearchResult, Square, Variant,
};
use std::f32::consts::PI;

const DEFAULT_SEARCH_DEPTH: u8 = 3;
const MIN_SEARCH_DEPTH: u8 = 1;
const MAX_SEARCH_DEPTH: u8 = 6;
const MOVE_ANIMATION_SECONDS: f32 = 0.38;
const PIECE_MODEL_SCALE: f32 = 0.018;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Capablanca Chess Plus 3D".into(),
                name: Some("capablanca-chess-plus-3d".into()),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((MeshPickingPlugin, PanOrbitCameraPlugin))
        .init_resource::<ChessMatch>()
        .init_resource::<AiSettings>()
        .init_resource::<AiTask>()
        .init_resource::<RenderedPosition>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                handle_keyboard,
                poll_ai_task,
                start_ai_task,
                sync_board_geometry,
                sync_pieces,
                update_square_materials,
                animate_pieces,
                update_hud,
            )
                .chain(),
        )
        .run();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Controller {
    Human,
    Computer,
}

impl Controller {
    const fn label(self) -> &'static str {
        match self {
            Self::Human => "Human",
            Self::Computer => "Computer",
        }
    }

    fn toggle(&mut self) {
        *self = match self {
            Self::Human => Self::Computer,
            Self::Computer => Self::Human,
        };
    }
}

#[derive(Resource)]
struct ChessMatch {
    game: Game,
    variant: Variant,
    controllers: [Controller; 2],
    selected: Option<Square>,
    pending_promotion: Option<PendingPromotion>,
    last_move: Option<Move>,
    status: String,
    generation: u64,
}

impl Default for ChessMatch {
    fn default() -> Self {
        let variant = Variant::Capablanca;
        Self {
            game: Game::new(variant.starting_position()),
            variant,
            controllers: [Controller::Human, Controller::Computer],
            selected: None,
            pending_promotion: None,
            last_move: None,
            status: "White to move.".to_owned(),
            generation: 0,
        }
    }
}

#[derive(Clone)]
struct PendingPromotion {
    moves: Vec<Move>,
}

#[derive(Resource)]
struct AiSettings {
    depth: u8,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            depth: DEFAULT_SEARCH_DEPTH,
        }
    }
}

#[derive(Resource, Default)]
struct AiTask(Option<Task<AiReply>>);

struct AiReply {
    generation: u64,
    result: Option<SearchResult>,
}

#[derive(Resource)]
struct RenderedPosition {
    generation: u64,
    board_size: Option<BoardSize>,
}

impl Default for RenderedPosition {
    fn default() -> Self {
        Self {
            generation: u64::MAX,
            board_size: None,
        }
    }
}

#[derive(Resource)]
struct SceneAssets {
    square_mesh: Handle<Mesh>,
    frame_mesh: Handle<Mesh>,
    pawn_mesh: Handle<Mesh>,
    knight_mesh: Handle<Mesh>,
    bishop_mesh: Handle<Mesh>,
    rook_mesh: Handle<Mesh>,
    queen_mesh: Handle<Mesh>,
    king_mesh: Handle<Mesh>,
    archbishop_mesh: Handle<Mesh>,
    chancellor_mesh: Handle<Mesh>,
    light_square: Handle<StandardMaterial>,
    dark_square: Handle<StandardMaterial>,
    selected_square: Handle<StandardMaterial>,
    legal_square: Handle<StandardMaterial>,
    capture_square: Handle<StandardMaterial>,
    last_square: Handle<StandardMaterial>,
    check_square: Handle<StandardMaterial>,
    frame_material: Handle<StandardMaterial>,
    white_piece: Handle<StandardMaterial>,
    black_piece: Handle<StandardMaterial>,
}

#[derive(Component, Clone, Copy)]
struct BoardSquare(Square);

#[derive(Component)]
struct BoardFrame;

#[derive(Component)]
struct PieceRoot;

#[derive(Component)]
struct PieceMotion {
    start: Vec3,
    end: Vec3,
    elapsed: f32,
}

#[derive(Component)]
struct HudText;

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let assets = SceneAssets {
        square_mesh: meshes.add(Cuboid::new(0.98, 0.12, 0.98)),
        frame_mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        pawn_mesh: asset_server.load("models/pawn.glb#Mesh0/Primitive0"),
        knight_mesh: asset_server.load("models/knight.glb#Mesh0/Primitive0"),
        bishop_mesh: asset_server.load("models/bishop.glb#Mesh0/Primitive0"),
        rook_mesh: asset_server.load("models/rook.glb#Mesh0/Primitive0"),
        queen_mesh: asset_server.load("models/queen.glb#Mesh0/Primitive0"),
        king_mesh: asset_server.load("models/king.glb#Mesh0/Primitive0"),
        archbishop_mesh: asset_server.load("models/archbishop.glb#Mesh0/Primitive0"),
        chancellor_mesh: asset_server.load("models/chancellor.glb#Mesh0/Primitive0"),
        light_square: materials.add(chess_material(Color::srgb(0.72, 0.52, 0.31), 0.82)),
        dark_square: materials.add(chess_material(Color::srgb(0.24, 0.105, 0.055), 0.9)),
        selected_square: materials.add(highlight_material(
            Color::srgb(1.0, 0.66, 0.08),
            LinearRgba::rgb(0.7, 0.28, 0.01),
        )),
        legal_square: materials.add(highlight_material(
            Color::srgb(0.23, 0.72, 0.37),
            LinearRgba::rgb(0.02, 0.35, 0.05),
        )),
        capture_square: materials.add(highlight_material(
            Color::srgb(0.88, 0.22, 0.2),
            LinearRgba::rgb(0.45, 0.015, 0.01),
        )),
        last_square: materials.add(highlight_material(
            Color::srgb(0.2, 0.52, 0.9),
            LinearRgba::rgb(0.015, 0.12, 0.45),
        )),
        check_square: materials.add(highlight_material(
            Color::srgb(0.86, 0.04, 0.08),
            LinearRgba::rgb(0.8, 0.0, 0.01),
        )),
        frame_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.075, 0.025, 0.012),
            perceptual_roughness: 0.62,
            metallic: 0.08,
            ..default()
        }),
        white_piece: materials.add(StandardMaterial {
            base_color: Color::srgb(0.92, 0.82, 0.66),
            perceptual_roughness: 0.36,
            metallic: 0.04,
            ..default()
        }),
        black_piece: materials.add(StandardMaterial {
            base_color: Color::srgb(0.075, 0.055, 0.05),
            perceptual_roughness: 0.3,
            metallic: 0.16,
            ..default()
        }),
    };
    commands.insert_resource(assets);
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.78, 0.84, 1.0),
        brightness: 260.0,
        ..default()
    });

    commands.spawn((
        DirectionalLight {
            illuminance: 9_500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.55, 0.0)),
    ));

    commands.spawn((
        Transform::from_xyz(0.0, 10.5, -13.5).looking_at(Vec3::ZERO, Vec3::Y),
        PanOrbitCamera {
            button_orbit: MouseButton::Middle,
            button_pan: MouseButton::Right,
            zoom_lower_limit: 6.0,
            zoom_upper_limit: Some(25.0),
            ..default()
        },
    ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(16),
                right: px(16),
                width: px(350),
                padding: UiRect::all(px(16)),
                border_radius: BorderRadius::all(px(10)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.03, 0.045, 0.9)),
            Pickable::IGNORE,
        ))
        .with_child((
            Text::new(""),
            TextFont {
                font: asset_server.load("fonts/FiraSans-Bold.ttf").into(),
                font_size: FontSize::Px(17.0),
                ..default()
            },
            TextColor(Color::srgb(0.92, 0.94, 1.0)),
            Pickable::IGNORE,
            HudText,
        ));
}

fn chess_material(color: Color, roughness: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        perceptual_roughness: roughness,
        ..default()
    }
}

fn highlight_material(color: Color, emissive: LinearRgba) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        emissive,
        perceptual_roughness: 0.68,
        ..default()
    }
}

fn handle_keyboard(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut chess_match: ResMut<ChessMatch>,
    mut ai_settings: ResMut<AiSettings>,
    mut ai_task: ResMut<AiTask>,
) {
    if chess_match.pending_promotion.is_some() {
        if keyboard.just_pressed(KeyCode::Escape) {
            chess_match.pending_promotion = None;
            chess_match.selected = None;
            chess_match.status = "Promotion cancelled.".to_owned();
            return;
        }

        let requested = promotion_key(&keyboard);
        if requested.is_some() || keyboard.just_pressed(KeyCode::Space) {
            let promotion = requested.flatten();
            let chess_move = chess_match.pending_promotion.as_ref().and_then(|pending| {
                pending
                    .moves
                    .iter()
                    .copied()
                    .find(|candidate| candidate.promotion == promotion)
            });
            if let Some(chess_move) = chess_move {
                apply_move(&mut chess_match, chess_move, None);
            } else {
                chess_match.status = promotion_prompt(
                    &chess_match
                        .pending_promotion
                        .as_ref()
                        .expect("promotion is pending")
                        .moves,
                );
            }
            return;
        }
    }

    if keyboard.just_pressed(KeyCode::Escape) {
        chess_match.selected = None;
        chess_match.status = side_to_move_message(&chess_match);
    }

    if keyboard.just_pressed(KeyCode::KeyN) || keyboard.just_pressed(KeyCode::Numpad0) {
        let variant = chess_match.variant;
        restart_match(&mut chess_match, variant);
        ai_task.0 = None;
    }

    if let Some(variant) = variant_key(&keyboard)
        && variant != chess_match.variant
    {
        restart_match(&mut chess_match, variant);
        ai_task.0 = None;
    }

    let toggle_white =
        keyboard.just_pressed(KeyCode::Digit1) || keyboard.just_pressed(KeyCode::Numpad1);
    let toggle_black =
        keyboard.just_pressed(KeyCode::Digit2) || keyboard.just_pressed(KeyCode::Numpad2);
    if toggle_white || toggle_black {
        let index = usize::from(toggle_black);
        chess_match.controllers[index].toggle();
        chess_match.selected = None;
        chess_match.pending_promotion = None;
        chess_match.status = format!(
            "{} is now controlled by the {}.",
            side_name(if index == 0 { Side::White } else { Side::Black }),
            chess_match.controllers[index].label().to_ascii_lowercase()
        );
        ai_task.0 = None;
    }

    let increase_depth = keyboard.just_pressed(KeyCode::Equal)
        || keyboard.just_pressed(KeyCode::NumpadAdd)
        || keyboard.just_pressed(KeyCode::ArrowUp);
    let decrease_depth = keyboard.just_pressed(KeyCode::Minus)
        || keyboard.just_pressed(KeyCode::NumpadSubtract)
        || keyboard.just_pressed(KeyCode::ArrowDown);
    let old_depth = ai_settings.depth;
    if increase_depth {
        ai_settings.depth = ai_settings.depth.saturating_add(1).min(MAX_SEARCH_DEPTH);
    }
    if decrease_depth {
        ai_settings.depth = ai_settings.depth.saturating_sub(1).max(MIN_SEARCH_DEPTH);
    }
    if ai_settings.depth != old_depth {
        chess_match.status = format!("Engine search depth set to {}.", ai_settings.depth);
        ai_task.0 = None;
    }
}

fn promotion_key(keyboard: &ButtonInput<KeyCode>) -> Option<Option<PieceKind>> {
    [
        (KeyCode::KeyQ, PieceKind::Queen),
        (KeyCode::KeyC, PieceKind::Chancellor),
        (KeyCode::KeyA, PieceKind::Archbishop),
        (KeyCode::KeyR, PieceKind::Rook),
        (KeyCode::KeyB, PieceKind::Bishop),
        (KeyCode::KeyN, PieceKind::Knight),
    ]
    .into_iter()
    .find_map(|(key, kind)| keyboard.just_pressed(key).then_some(Some(kind)))
}

fn variant_key(keyboard: &ButtonInput<KeyCode>) -> Option<Variant> {
    [
        (KeyCode::F1, Variant::Capablanca),
        (KeyCode::F2, Variant::Gothic),
        (KeyCode::F3, Variant::Embassy),
        (KeyCode::F4, Variant::Schoolbook),
        (KeyCode::F5, Variant::Bird),
        (KeyCode::F6, Variant::Carrera),
        (KeyCode::F7, Variant::Grand),
    ]
    .into_iter()
    .find_map(|(key, variant)| keyboard.just_pressed(key).then_some(variant))
}

fn restart_match(chess_match: &mut ChessMatch, variant: Variant) {
    chess_match.game = Game::new(variant.starting_position());
    chess_match.variant = variant;
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    chess_match.last_move = None;
    chess_match.status = format!("New {} game. White to move.", variant.rules().name());
    chess_match.generation = chess_match.generation.wrapping_add(1);
}

fn on_square_click(
    click: On<Pointer<Click>>,
    squares: Query<&BoardSquare>,
    mut chess_match: ResMut<ChessMatch>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok(clicked) = squares.get(click.entity) else {
        return;
    };
    if chess_match.pending_promotion.is_some() {
        return;
    }
    if !is_playable(chess_match.game.outcome()) {
        chess_match.status = outcome_message(chess_match.game.outcome());
        return;
    }

    let side = chess_match.game.position().side_to_move();
    if chess_match.controllers[side.index()] == Controller::Computer {
        chess_match.status = format!("{} is controlled by the computer.", side_name(side));
        return;
    }

    let square = clicked.0;
    if let Some(from) = chess_match.selected {
        let candidates: Vec<_> = chess_match
            .game
            .position()
            .legal_moves()
            .into_iter()
            .filter(|chess_move| chess_move.from == from && chess_move.to == square)
            .collect();
        match candidates.as_slice() {
            [] => {
                if selectable_piece(&chess_match, square) {
                    select_square(&mut chess_match, square);
                } else {
                    chess_match.selected = None;
                    chess_match.status = format!("{square} is not a legal destination.");
                }
            }
            [chess_move] => apply_move(&mut chess_match, *chess_move, None),
            _ => {
                chess_match.status = promotion_prompt(&candidates);
                chess_match.pending_promotion = Some(PendingPromotion { moves: candidates });
            }
        }
    } else if selectable_piece(&chess_match, square) {
        select_square(&mut chess_match, square);
    } else {
        chess_match.status = format!("Select a {} piece.", side_name(side).to_ascii_lowercase());
    }
}

fn selectable_piece(chess_match: &ChessMatch, square: Square) -> bool {
    let position = chess_match.game.position();
    position
        .board()
        .piece_at(square)
        .is_some_and(|piece| piece.color == position.side_to_move())
        && position
            .legal_moves()
            .iter()
            .any(|chess_move| chess_move.from == square)
}

fn select_square(chess_match: &mut ChessMatch, square: Square) {
    chess_match.selected = Some(square);
    let count = chess_match
        .game
        .position()
        .legal_moves()
        .iter()
        .filter(|chess_move| chess_move.from == square)
        .map(|chess_move| chess_move.to)
        .collect::<std::collections::HashSet<_>>()
        .len();
    chess_match.status = format!("{square} selected: {count} destination(s).");
}

fn apply_move(chess_match: &mut ChessMatch, chess_move: Move, analysis: Option<&SearchResult>) {
    let position = chess_match.game.position();
    let moving_piece = position
        .board()
        .piece_at(chess_move.from)
        .expect("a legal move has a source piece");
    let is_capture = matches!(chess_move.kind, MoveKind::EnPassant)
        || position.board().piece_at(chess_move.to).is_some();
    let description = describe_move(moving_piece, chess_move, is_capture);

    chess_match
        .game
        .play(chess_move)
        .expect("only engine-provided legal moves are applied");
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    chess_match.last_move = Some(chess_move);
    chess_match.generation = chess_match.generation.wrapping_add(1);

    let analysis_text = analysis.map_or_else(String::new, |result| {
        format!(
            "  Evaluation {:+.2}, depth {}, {} nodes.",
            f64::from(result.score) / 100.0,
            result.depth,
            result.nodes
        )
    });
    let outcome = chess_match.game.outcome();
    chess_match.status = format!("{description}.{analysis_text} {}", outcome_message(outcome));
}

fn describe_move(piece: Piece, chess_move: Move, capture: bool) -> String {
    if let MoveKind::Castle(side) = chess_move.kind {
        return format!(
            "{} castles {}",
            side_name(piece.color),
            match side {
                CastleSide::QueenSide => "queen-side",
                CastleSide::KingSide => "king-side",
            }
        );
    }
    let separator = if capture { "x" } else { "-" };
    let mut value = format!(
        "{} {} {}{}{}",
        side_name(piece.color),
        piece_name(piece.kind),
        chess_move.from,
        separator,
        chess_move.to
    );
    if let Some(promotion) = chess_move.promotion {
        value.push_str(&format!(" promotes to {}", piece_name(promotion)));
    }
    value
}

fn start_ai_task(
    mut chess_match: ResMut<ChessMatch>,
    settings: Res<AiSettings>,
    mut task: ResMut<AiTask>,
) {
    if task.0.is_some()
        || chess_match.pending_promotion.is_some()
        || !is_playable(chess_match.game.outcome())
    {
        return;
    }
    let side = chess_match.game.position().side_to_move();
    if chess_match.controllers[side.index()] != Controller::Computer {
        return;
    }

    let position = chess_match.game.position().clone();
    let generation = chess_match.generation;
    let depth = settings.depth;
    chess_match.status = format!("{} computer is thinking at depth {depth}…", side_name(side));
    task.0 = Some(AsyncComputeTaskPool::get().spawn(async move {
        let result = Engine::new().search(&position, SearchLimits::depth(depth));
        AiReply { generation, result }
    }));
}

fn poll_ai_task(mut chess_match: ResMut<ChessMatch>, mut task: ResMut<AiTask>) {
    let Some(ai_task) = task.0.as_mut() else {
        return;
    };
    let Some(reply) = future::block_on(future::poll_once(ai_task)) else {
        return;
    };
    task.0 = None;

    if reply.generation != chess_match.generation {
        return;
    }
    if let Some(result) = reply.result {
        apply_move(&mut chess_match, result.best_move, Some(&result));
    } else {
        chess_match.status = outcome_message(chess_match.game.outcome());
    }
}

fn sync_board_geometry(
    mut commands: Commands,
    chess_match: Res<ChessMatch>,
    assets: Res<SceneAssets>,
    mut rendered: ResMut<RenderedPosition>,
    squares: Query<Entity, With<BoardSquare>>,
    frames: Query<Entity, With<BoardFrame>>,
    mut cameras: Query<&mut PanOrbitCamera>,
) {
    let position = chess_match.game.position();
    let size = position.board().size();

    if rendered.board_size != Some(size) {
        for entity in &squares {
            commands.entity(entity).despawn();
        }
        for entity in &frames {
            commands.entity(entity).despawn();
        }
        spawn_board(&mut commands, &assets, size);
        if let Ok(mut camera) = cameras.single_mut() {
            let radius = f32::from(size.files().max(size.ranks())) * 1.42;
            camera.target_focus = Vec3::ZERO;
            camera.target_radius = radius;
            camera.zoom_upper_limit = Some(radius * 1.8);
            camera.force_update = true;
        }
        rendered.board_size = Some(size);
    }
}

fn sync_pieces(
    mut commands: Commands,
    chess_match: Res<ChessMatch>,
    assets: Res<SceneAssets>,
    mut rendered: ResMut<RenderedPosition>,
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

fn spawn_board(commands: &mut Commands, assets: &SceneAssets, size: BoardSize) {
    commands.spawn((
        Mesh3d(assets.frame_mesh.clone()),
        MeshMaterial3d(assets.frame_material.clone()),
        Transform::from_xyz(0.0, -0.15, 0.0).with_scale(Vec3::new(
            f32::from(size.files()) + 0.7,
            0.18,
            f32::from(size.ranks()) + 0.7,
        )),
        Pickable::IGNORE,
        BoardFrame,
    ));

    for rank in 0..size.ranks() {
        for file in 0..size.files() {
            let square = Square::new(file, rank);
            let material = if (file + rank) % 2 == 0 {
                assets.dark_square.clone()
            } else {
                assets.light_square.clone()
            };
            commands
                .spawn((
                    Mesh3d(assets.square_mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(square_world(square, size) - Vec3::Y * 0.06),
                    BoardSquare(square),
                ))
                .observe(on_square_click);
        }
    }
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
    assets: &SceneAssets,
    square: Square,
    piece: Piece,
    start: Vec3,
    target: Vec3,
) {
    let material = match piece.color {
        Side::White => assets.white_piece.clone(),
        Side::Black => assets.black_piece.clone(),
    };
    let model = piece_model(piece.kind, assets);
    let mut entity = commands.spawn((
        Transform::from_translation(start).with_rotation(Quat::from_rotation_y(
            if piece.color == Side::Black { PI } else { 0.0 },
        )),
        Visibility::default(),
        PieceRoot,
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
    entity.with_child((
        Mesh3d(model.mesh),
        MeshMaterial3d(material),
        model.transform,
        Pickable::IGNORE,
    ));
}

struct PieceModel {
    mesh: Handle<Mesh>,
    transform: Transform,
}

fn piece_model(kind: PieceKind, assets: &SceneAssets) -> PieceModel {
    let (mesh, center_x, min_y, center_z) = match kind {
        PieceKind::Pawn => (&assets.pawn_mesh, -0.005_807, 0.0, -0.010_721),
        PieceKind::Knight => (&assets.knight_mesh, 15.440_479, 0.0, 4.078_164),
        PieceKind::Bishop => (&assets.bishop_mesh, -0.040_336, -21.983_118, 0.098_973),
        PieceKind::Rook => (&assets.rook_mesh, -22.567_917, 0.0, -2.458_724),
        PieceKind::Queen => (&assets.queen_mesh, 0.034_510, -34.450_596, 0.070_159),
        PieceKind::King => (&assets.king_mesh, 11.008_427, 0.0, -1.965_424),
        PieceKind::Archbishop => (&assets.archbishop_mesh, 0.060_687, -27.845_299, -0.058_388),
        PieceKind::Chancellor => (&assets.chancellor_mesh, -0.008_492, -24.878_624, -0.414_659),
    };
    let rotation = if kind == PieceKind::Knight {
        Quat::from_rotation_y(PI)
    } else {
        Quat::IDENTITY
    };
    let anchor = Vec3::new(center_x, min_y, center_z) * PIECE_MODEL_SCALE;

    PieceModel {
        mesh: mesh.clone(),
        transform: Transform::from_translation(-(rotation * anchor))
            .with_rotation(rotation)
            .with_scale(Vec3::splat(PIECE_MODEL_SCALE)),
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

fn update_square_materials(
    chess_match: Res<ChessMatch>,
    assets: Res<SceneAssets>,
    mut squares: Query<(&BoardSquare, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let position = chess_match.game.position();
    let selected_moves = chess_match.selected.map_or_else(Vec::new, |selected| {
        position
            .legal_moves()
            .into_iter()
            .filter(|chess_move| chess_move.from == selected)
            .collect()
    });
    let checked_king = (chess_match.game.outcome() == GameOutcome::Check)
        .then(|| position.board().king_square(position.side_to_move()))
        .flatten();

    for (board_square, mut material) in &mut squares {
        let square = board_square.0;
        material.0 = if checked_king == Some(square) {
            assets.check_square.clone()
        } else if chess_match.selected == Some(square) {
            assets.selected_square.clone()
        } else if let Some(chess_move) = selected_moves
            .iter()
            .find(|chess_move| chess_move.to == square)
        {
            if matches!(chess_move.kind, MoveKind::EnPassant)
                || position.board().piece_at(square).is_some()
            {
                assets.capture_square.clone()
            } else {
                assets.legal_square.clone()
            }
        } else if chess_match
            .last_move
            .is_some_and(|last| last.from == square || last.to == square)
        {
            assets.last_square.clone()
        } else if (square.file() + square.rank()) % 2 == 0 {
            assets.dark_square.clone()
        } else {
            assets.light_square.clone()
        };
    }
}

fn update_hud(
    chess_match: Res<ChessMatch>,
    settings: Res<AiSettings>,
    mut hud: Query<&mut Text, With<HudText>>,
) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };
    let position = chess_match.game.position();
    let size = position.board().size();
    let selected = chess_match
        .selected
        .map_or_else(|| "none".to_owned(), |square| square.to_string());
    **text = format!(
        "{}\n\
         Board: {} × {}\n\
         White: {}    Black: {}\n\
         Engine depth: {}\n\
         Side to move: {}\n\
         Selected: {}\n\n\
         {}\n\n\
         Left click: select / move\n\
         Middle drag: orbit    Right drag: pan\n\
         Wheel: zoom    Esc: cancel\n\
         1 / 2: toggle human or computer\n\
         ↑ / ↓ or + / −: engine depth\n\
         N: new game\n\
         F1 Capablanca  F2 Gothic\n\
         F3 Embassy     F4 Schoolbook\n\
         F5 Bird        F6 Carrera\n\
         F7 Grand Chess\n\n\
         Promotion: Q/C/A/R/B/N\n\
         (Space = no promotion where allowed)",
        position.rules().name(),
        size.files(),
        size.ranks(),
        chess_match.controllers[Side::White.index()].label(),
        chess_match.controllers[Side::Black.index()].label(),
        settings.depth,
        side_name(position.side_to_move()),
        selected,
        chess_match.status,
    );
}

fn square_world(square: Square, size: BoardSize) -> Vec3 {
    Vec3::new(
        f32::from(square.file()) - (f32::from(size.files()) - 1.0) * 0.5,
        0.0,
        f32::from(square.rank()) - (f32::from(size.ranks()) - 1.0) * 0.5,
    )
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::White => "White",
        Side::Black => "Black",
    }
}

fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "pawn",
        PieceKind::Knight => "knight",
        PieceKind::Bishop => "bishop",
        PieceKind::Rook => "rook",
        PieceKind::Queen => "queen",
        PieceKind::King => "king",
        PieceKind::Archbishop => "archbishop",
        PieceKind::Chancellor => "chancellor",
    }
}

fn side_to_move_message(chess_match: &ChessMatch) -> String {
    let side = chess_match.game.position().side_to_move();
    format!("{} to move.", side_name(side))
}

fn is_playable(outcome: GameOutcome) -> bool {
    matches!(outcome, GameOutcome::Ongoing | GameOutcome::Check)
}

fn outcome_message(outcome: GameOutcome) -> String {
    match outcome {
        GameOutcome::Ongoing => "Game in progress.".to_owned(),
        GameOutcome::Check => "Check.".to_owned(),
        GameOutcome::Win { winner } => {
            format!("Checkmate — {} wins.", side_name(winner))
        }
        GameOutcome::Draw(reason) => match reason {
            DrawReason::Stalemate => "Draw by stalemate.".to_owned(),
            DrawReason::FiftyMoveRule => "Draw by the fifty-move rule.".to_owned(),
            DrawReason::ThreefoldRepetition => "Draw by threefold repetition.".to_owned(),
        },
    }
}

fn promotion_prompt(moves: &[Move]) -> String {
    let mut choices = Vec::new();
    for chess_move in moves {
        let choice = match chess_move.promotion {
            Some(PieceKind::Queen) => "Q queen",
            Some(PieceKind::Chancellor) => "C chancellor",
            Some(PieceKind::Archbishop) => "A archbishop",
            Some(PieceKind::Rook) => "R rook",
            Some(PieceKind::Bishop) => "B bishop",
            Some(PieceKind::Knight) => "N knight",
            Some(PieceKind::Pawn | PieceKind::King) => continue,
            None => "Space no promotion",
        };
        if !choices.contains(&choice) {
            choices.push(choice);
        }
    }
    format!("Choose promotion: {}.", choices.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_world_centers_both_supported_board_sizes() {
        for size in [BoardSize::CAPABLANCA, BoardSize::GRAND] {
            let lower = square_world(Square::new(0, 0), size);
            let upper = square_world(Square::new(size.files() - 1, size.ranks() - 1), size);
            assert_eq!(lower + upper, Vec3::ZERO);
        }
    }

    #[test]
    fn promotion_prompt_exposes_compound_piece_keys() {
        let from = Square::new(0, 6);
        let to = Square::new(0, 7);
        let moves = [
            Move::promotion(from, to, PieceKind::Queen),
            Move::promotion(from, to, PieceKind::Chancellor),
            Move::promotion(from, to, PieceKind::Archbishop),
        ];
        let prompt = promotion_prompt(&moves);
        assert!(prompt.contains("Q queen"));
        assert!(prompt.contains("C chancellor"));
        assert!(prompt.contains("A archbishop"));
    }
}
