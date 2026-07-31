use bevy::{
    color::LinearRgba,
    ecs::system::SystemParam,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    mesh::VertexAttributeValues,
    prelude::*,
};
use bevy_panorbit_camera::PanOrbitCamera;
use capablanca_chess_plus::{BoardSize, GameOutcome, MoveKind, Square};

use crate::{
    app::FrontendSet,
    game::{ChessMatch, handle_square_selection},
};

// The source maps have different average levels. These factors keep both marble
// colors near the same polished-but-stable roughness and prevent normal-map
// details from turning into mirror-like speckles. Order: black, white.
const MARBLE_ROUGHNESS_FACTORS: [f32; 2] = [1.6, 2.4];
const WOOD_ROUGHNESS_FACTOR: f32 = 1.45;
const WOOD_TEXTURE_WORLD_SIZE: f32 = 3.0;

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

#[derive(SystemParam)]
struct BoardScene<'w, 's> {
    squares: Query<'w, 's, Entity, With<BoardSquare>>,
    parts: Query<'w, 's, Entity, With<BoardPart>>,
    cameras: Query<'w, 's, &'static mut PanOrbitCamera>,
}

fn setup_board_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let white_marble = asset_server.load("textures/white_marble_color.jpg");
    let black_marble = asset_server.load("textures/black_marble_color.jpg");
    let marble_textures = [&black_marble, &white_marble];
    let white_marble_normal =
        load_linear_texture(&asset_server, "textures/white_marble_normalgl.jpg");
    let black_marble_normal =
        load_linear_texture(&asset_server, "textures/black_marble_normalgl.jpg");
    let marble_normals = [&black_marble_normal, &white_marble_normal];
    let white_marble_roughness =
        load_linear_texture(&asset_server, "textures/white_marble_roughness.jpg");
    let black_marble_roughness =
        load_linear_texture(&asset_server, "textures/black_marble_roughness.jpg");
    let marble_roughness = [&black_marble_roughness, &white_marble_roughness];
    let wood_color = load_repeating_texture(&asset_server, "textures/wood_color.jpg", true);
    let wood_normal = load_repeating_texture(&asset_server, "textures/wood_normalgl.jpg", false);
    let wood_roughness =
        load_repeating_texture(&asset_server, "textures/wood_roughness.jpg", false);

    let square_materials = SquareMaterials {
        normal: marble_material_pair(
            &mut materials,
            marble_textures,
            marble_normals,
            marble_roughness,
            Color::WHITE,
            LinearRgba::BLACK,
        ),
        selected: marble_material_pair(
            &mut materials,
            marble_textures,
            marble_normals,
            marble_roughness,
            Color::srgb(1.0, 0.66, 0.08),
            LinearRgba::rgb(0.7, 0.28, 0.01),
        ),
        legal: marble_material_pair(
            &mut materials,
            marble_textures,
            marble_normals,
            marble_roughness,
            Color::srgb(0.23, 0.72, 0.37),
            LinearRgba::rgb(0.02, 0.35, 0.05),
        ),
        capture: marble_material_pair(
            &mut materials,
            marble_textures,
            marble_normals,
            marble_roughness,
            Color::srgb(0.88, 0.22, 0.2),
            LinearRgba::rgb(0.45, 0.015, 0.01),
        ),
        last: marble_material_pair(
            &mut materials,
            marble_textures,
            marble_normals,
            marble_roughness,
            Color::srgb(0.2, 0.52, 0.9),
            LinearRgba::rgb(0.015, 0.12, 0.45),
        ),
        check: marble_material_pair(
            &mut materials,
            marble_textures,
            marble_normals,
            marble_roughness,
            Color::srgb(0.86, 0.04, 0.08),
            LinearRgba::rgb(0.8, 0.0, 0.01),
        ),
    };

    commands.insert_resource(BoardAssets {
        square_materials,
        wood_material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(wood_color),
            normal_map_texture: Some(wood_normal),
            metallic_roughness_texture: Some(wood_roughness),
            perceptual_roughness: WOOD_ROUGHNESS_FACTOR,
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
    normal_maps: [&Handle<Image>; 2],
    roughness_maps: [&Handle<Image>; 2],
    tint: Color,
    emissive: LinearRgba,
) -> [Handle<StandardMaterial>; 2] {
    std::array::from_fn(|index| {
        materials.add(StandardMaterial {
            base_color: tint,
            base_color_texture: Some(color_textures[index].clone()),
            normal_map_texture: Some(normal_maps[index].clone()),
            metallic_roughness_texture: Some(roughness_maps[index].clone()),
            emissive,
            metallic: 0.0,
            perceptual_roughness: MARBLE_ROUGHNESS_FACTORS[index],
            reflectance: 0.45,
            clearcoat: 0.25,
            clearcoat_perceptual_roughness: 0.18,
            ..default()
        })
    })
}

fn load_linear_texture(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| settings.is_srgb = false)
        .load(path)
}

fn load_repeating_texture(
    asset_server: &AssetServer,
    path: &'static str,
    is_srgb: bool,
) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(move |settings: &mut ImageLoaderSettings| {
            settings.is_srgb = is_srgb;
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                ..ImageSamplerDescriptor::linear()
            });
        })
        .load(path)
}

fn sync_board_geometry(
    mut commands: Commands,
    chess_match: Res<ChessMatch>,
    assets: Res<BoardAssets>,
    mut rendered: ResMut<BoardRenderState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut scene: BoardScene,
) {
    let size = chess_match.game.position().board().size();
    if rendered.size == Some(size) {
        return;
    }

    for entity in &scene.squares {
        commands.entity(entity).despawn();
    }
    for entity in &scene.parts {
        commands.entity(entity).despawn();
    }
    spawn_board(&mut commands, &assets, &mut meshes, size);

    if let Ok(mut camera) = scene.cameras.single_mut() {
        let radius = f32::from(size.files().max(size.ranks())) * 1.42;
        camera.target_focus = Vec3::ZERO;
        camera.target_radius = radius;
        camera.zoom_upper_limit = Some(radius * 1.8);
        camera.force_update = true;
    }
    rendered.size = Some(size);
}

fn spawn_board(
    commands: &mut Commands,
    assets: &BoardAssets,
    meshes: &mut Assets<Mesh>,
    size: BoardSize,
) {
    let width = f32::from(size.files());
    let depth = f32::from(size.ranks());
    let outer_width = width + 0.8;
    let outer_depth = depth + 0.8;

    let base_mesh = meshes.add(wood_mesh(Vec3::new(outer_width, 0.18, outer_depth)));
    spawn_wood_part(
        commands,
        assets,
        base_mesh,
        Transform::from_xyz(0.0, -0.2, 0.0),
    );

    let horizontal_rail = meshes.add(wood_mesh(Vec3::new(outer_width, 0.16, 0.4)));
    for z in [-depth * 0.5 - 0.2, depth * 0.5 + 0.2] {
        spawn_wood_part(
            commands,
            assets,
            horizontal_rail.clone(),
            Transform::from_xyz(0.0, -0.04, z),
        );
    }

    let vertical_rail = meshes.add(wood_mesh(Vec3::new(depth, 0.16, 0.4)));
    for x in [-width * 0.5 - 0.2, width * 0.5 + 0.2] {
        spawn_wood_part(
            commands,
            assets,
            vertical_rail.clone(),
            Transform::from_xyz(x, -0.04, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        );
    }

    for rank in 0..size.ranks() {
        for file in 0..size.files() {
            let square = Square::new(file, rank);
            let material = assets.square_materials.normal[material_index(square)].clone();
            commands
                .spawn((
                    Mesh3d(meshes.add(square_mesh(square, size))),
                    MeshMaterial3d(material),
                    Transform::from_translation(square_world(square, size) - Vec3::Y * 0.06),
                    BoardSquare(square),
                ))
                .observe(on_square_click);
        }
    }
}

fn spawn_wood_part(
    commands: &mut Commands,
    assets: &BoardAssets,
    mesh: Handle<Mesh>,
    transform: Transform,
) {
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(assets.wood_material.clone()),
        transform,
        Pickable::IGNORE,
        BoardPart,
    ));
}

fn square_mesh(square: Square, size: BoardSize) -> Mesh {
    let mut mesh = Mesh::from(Cuboid::new(0.98, 0.12, 0.98));
    let VertexAttributeValues::Float32x2(uvs) = mesh
        .attribute_mut(Mesh::ATTRIBUTE_UV_0)
        .expect("a cuboid has UV coordinates")
    else {
        unreachable!("cuboid UV coordinates use Float32x2");
    };

    let files = f32::from(size.files());
    let ranks = f32::from(size.ranks());
    for uv in uvs {
        uv[0] = (f32::from(square.file()) + uv[0]) / files;
        uv[1] = (f32::from(square.rank()) + uv[1]) / ranks;
    }

    mesh.with_generated_tangents()
        .expect("a marble square has valid UVs for tangent generation")
}

fn wood_mesh(size: Vec3) -> Mesh {
    let mut mesh = Mesh::from(Cuboid::from_size(size));
    let positions = match mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .expect("a cuboid has vertex positions")
    {
        VertexAttributeValues::Float32x3(values) => values.clone(),
        _ => unreachable!("cuboid positions use Float32x3"),
    };
    let normals = match mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .expect("a cuboid has vertex normals")
    {
        VertexAttributeValues::Float32x3(values) => values.clone(),
        _ => unreachable!("cuboid normals use Float32x3"),
    };
    let VertexAttributeValues::Float32x2(uvs) = mesh
        .attribute_mut(Mesh::ATTRIBUTE_UV_0)
        .expect("a cuboid has UV coordinates")
    else {
        unreachable!("cuboid UV coordinates use Float32x2");
    };

    let half_size = size * 0.5;
    for ((uv, position), normal) in uvs.iter_mut().zip(positions).zip(normals) {
        let position = Vec3::from_array(position) + half_size;
        if normal[1].abs() > 0.5 {
            *uv = [position.x, position.z];
        } else if normal[2].abs() > 0.5 {
            *uv = [position.x, position.y];
        } else {
            *uv = [position.z, position.y];
        }
        uv[0] /= WOOD_TEXTURE_WORLD_SIZE;
        uv[1] /= WOOD_TEXTURE_WORLD_SIZE;
    }

    mesh.with_generated_tangents()
        .expect("a wooden board part has valid UVs for tangent generation")
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

    #[test]
    fn square_uvs_select_their_part_of_the_full_board_texture() {
        let size = BoardSize::CAPABLANCA;
        let square = Square::new(3, 5);
        let mesh = square_mesh(square, size);
        let VertexAttributeValues::Float32x2(uvs) = mesh
            .attribute(Mesh::ATTRIBUTE_UV_0)
            .expect("square mesh has UV coordinates")
        else {
            panic!("square mesh UV coordinates use an unexpected format");
        };

        let min = uvs.iter().fold(Vec2::splat(f32::INFINITY), |min, uv| {
            min.min(Vec2::from_array(*uv))
        });
        let max = uvs.iter().fold(Vec2::splat(f32::NEG_INFINITY), |max, uv| {
            max.max(Vec2::from_array(*uv))
        });

        assert_eq!(
            min,
            Vec2::new(
                f32::from(square.file()) / f32::from(size.files()),
                f32::from(square.rank()) / f32::from(size.ranks()),
            )
        );
        assert_eq!(
            max,
            Vec2::new(
                f32::from(square.file() + 1) / f32::from(size.files()),
                f32::from(square.rank() + 1) / f32::from(size.ranks()),
            )
        );
    }
}
