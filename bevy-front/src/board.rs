use bevy::{color::LinearRgba, image::ImageLoaderSettings, prelude::*};
use bevy_panorbit_camera::PanOrbitCamera;
use capablanca_chess_plus::{BoardSize, GameOutcome, MoveKind, Square};

use crate::{
    app::FrontendSet,
    game::{ChessMatch, handle_square_selection},
};

pub(crate) struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BoardRenderState>()
            .add_systems(Startup, setup_board_assets)
            .add_systems(Update, sync_board_geometry.in_set(FrontendSet::BoardSync))
            .add_systems(
                Update,
                update_square_materials.in_set(FrontendSet::Highlights),
            );
    }
}

#[derive(Resource)]
struct BoardAssets {
    square_mesh: Handle<Mesh>,
    board_part_mesh: Handle<Mesh>,
    square_materials: SquareMaterials,
    wood_material: Handle<StandardMaterial>,
}

struct SquareMaterials {
    normal: [Handle<StandardMaterial>; 2],
    selected: [Handle<StandardMaterial>; 2],
    legal: [Handle<StandardMaterial>; 2],
    capture: [Handle<StandardMaterial>; 2],
    last: [Handle<StandardMaterial>; 2],
    check: [Handle<StandardMaterial>; 2],
}

#[derive(Resource, Default)]
struct BoardRenderState {
    size: Option<BoardSize>,
}

#[derive(Component, Clone, Copy)]
struct BoardSquare(Square);

#[derive(Component)]
struct BoardPart;

fn setup_board_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let white_marble = asset_server.load("textures/white_marble_color.jpg");
    let black_marble = asset_server.load("textures/black_marble_color.jpg");
    let marble_textures = [&black_marble, &white_marble];
    let wood_color = asset_server.load("textures/wood_color.jpg");
    let wood_normal = asset_server
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| settings.is_srgb = false)
        .load("textures/wood_normalgl.jpg");

    let square_materials = SquareMaterials {
        normal: marble_material_pair(
            &mut materials,
            marble_textures,
            Color::WHITE,
            LinearRgba::BLACK,
        ),
        selected: marble_material_pair(
            &mut materials,
            marble_textures,
            Color::srgb(1.0, 0.66, 0.08),
            LinearRgba::rgb(0.7, 0.28, 0.01),
        ),
        legal: marble_material_pair(
            &mut materials,
            marble_textures,
            Color::srgb(0.23, 0.72, 0.37),
            LinearRgba::rgb(0.02, 0.35, 0.05),
        ),
        capture: marble_material_pair(
            &mut materials,
            marble_textures,
            Color::srgb(0.88, 0.22, 0.2),
            LinearRgba::rgb(0.45, 0.015, 0.01),
        ),
        last: marble_material_pair(
            &mut materials,
            marble_textures,
            Color::srgb(0.2, 0.52, 0.9),
            LinearRgba::rgb(0.015, 0.12, 0.45),
        ),
        check: marble_material_pair(
            &mut materials,
            marble_textures,
            Color::srgb(0.86, 0.04, 0.08),
            LinearRgba::rgb(0.8, 0.0, 0.01),
        ),
    };

    commands.insert_resource(BoardAssets {
        square_mesh: meshes.add(
            Mesh::from(Cuboid::new(0.98, 0.12, 0.98))
                .with_generated_tangents()
                .expect("a cuboid has valid UVs for tangent generation"),
        ),
        board_part_mesh: meshes.add(
            Mesh::from(Cuboid::new(1.0, 1.0, 1.0))
                .with_generated_tangents()
                .expect("a cuboid has valid UVs for tangent generation"),
        ),
        square_materials,
        wood_material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(wood_color),
            normal_map_texture: Some(wood_normal),
            perceptual_roughness: 0.48,
            reflectance: 0.4,
            clearcoat: 0.08,
            clearcoat_perceptual_roughness: 0.3,
            ..default()
        }),
    });
}

fn marble_material_pair(
    materials: &mut Assets<StandardMaterial>,
    color_textures: [&Handle<Image>; 2],
    tint: Color,
    emissive: LinearRgba,
) -> [Handle<StandardMaterial>; 2] {
    std::array::from_fn(|index| {
        materials.add(StandardMaterial {
            base_color: tint,
            base_color_texture: Some(color_textures[index].clone()),
            normal_map_texture: None,
            emissive,
            metallic: 0.0,
            perceptual_roughness: 0.36,
            reflectance: 0.45,
            clearcoat: 0.25,
            clearcoat_perceptual_roughness: 0.18,
            ..default()
        })
    })
}

fn sync_board_geometry(
    mut commands: Commands,
    chess_match: Res<ChessMatch>,
    assets: Res<BoardAssets>,
    mut rendered: ResMut<BoardRenderState>,
    squares: Query<Entity, With<BoardSquare>>,
    board_parts: Query<Entity, With<BoardPart>>,
    mut cameras: Query<&mut PanOrbitCamera>,
) {
    let size = chess_match.game.position().board().size();
    if rendered.size == Some(size) {
        return;
    }

    for entity in &squares {
        commands.entity(entity).despawn();
    }
    for entity in &board_parts {
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
    rendered.size = Some(size);
}

fn spawn_board(commands: &mut Commands, assets: &BoardAssets, size: BoardSize) {
    let width = f32::from(size.files());
    let depth = f32::from(size.ranks());
    let outer_width = width + 0.8;
    let outer_depth = depth + 0.8;

    spawn_wood_part(
        commands,
        assets,
        Vec3::new(0.0, -0.2, 0.0),
        Vec3::new(outer_width, 0.18, outer_depth),
    );
    for z in [-depth * 0.5 - 0.2, depth * 0.5 + 0.2] {
        spawn_wood_part(
            commands,
            assets,
            Vec3::new(0.0, -0.04, z),
            Vec3::new(outer_width, 0.16, 0.4),
        );
    }
    for x in [-width * 0.5 - 0.2, width * 0.5 + 0.2] {
        spawn_wood_part(
            commands,
            assets,
            Vec3::new(x, -0.04, 0.0),
            Vec3::new(0.4, 0.16, depth),
        );
    }

    for rank in 0..size.ranks() {
        for file in 0..size.files() {
            let square = Square::new(file, rank);
            let material = assets.square_materials.normal[material_index(square)].clone();
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

fn spawn_wood_part(commands: &mut Commands, assets: &BoardAssets, translation: Vec3, scale: Vec3) {
    commands.spawn((
        Mesh3d(assets.board_part_mesh.clone()),
        MeshMaterial3d(assets.wood_material.clone()),
        Transform::from_translation(translation).with_scale(scale),
        Pickable::IGNORE,
        BoardPart,
    ));
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
    handle_square_selection(&mut chess_match, clicked.0);
}

fn update_square_materials(
    chess_match: Res<ChessMatch>,
    assets: Res<BoardAssets>,
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
        let index = material_index(square);
        material.0 = if checked_king == Some(square) {
            assets.square_materials.check[index].clone()
        } else if chess_match.selected == Some(square) {
            assets.square_materials.selected[index].clone()
        } else if let Some(chess_move) = selected_moves
            .iter()
            .find(|chess_move| chess_move.to == square)
        {
            if matches!(chess_move.kind, MoveKind::EnPassant)
                || position.board().piece_at(square).is_some()
            {
                assets.square_materials.capture[index].clone()
            } else {
                assets.square_materials.legal[index].clone()
            }
        } else if chess_match
            .last_move
            .is_some_and(|last| last.from == square || last.to == square)
        {
            assets.square_materials.last[index].clone()
        } else {
            assets.square_materials.normal[index].clone()
        };
    }
}

fn material_index(square: Square) -> usize {
    usize::from(!(square.file() + square.rank()).is_multiple_of(2))
}

pub(crate) fn square_world(square: Square, size: BoardSize) -> Vec3 {
    Vec3::new(
        f32::from(square.file()) - (f32::from(size.files()) - 1.0) * 0.5,
        0.0,
        f32::from(square.rank()) - (f32::from(size.ranks()) - 1.0) * 0.5,
    )
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
}
