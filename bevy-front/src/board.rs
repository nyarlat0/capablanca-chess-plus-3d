use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::NotShadowCaster,
    mesh::VertexAttributeValues,
    pbr::ExtendedMaterial,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_panorbit_camera::PanOrbitCamera;
use capablanca_chess_plus::{BoardSize, GameOutcome, MoveKind, Square};

use crate::{
    app::FrontendSet,
    game::{ChessMatch, MoveRequest, handle_square_selection},
    menu::GameMenuState,
    pieces::PieceAnimationState,
    reflection::{
        PlanarBoardMaterial, PlanarReflectionExtension, PlanarReflectionImage,
        PlanarReflectionStartup,
    },
    render_tuning::{TEXTURE_ANISOTROPY, generated_surface_texture_path},
};

// The source maps have different average levels. These factors keep both marble
// colors near the same polished-but-stable roughness and prevent normal-map
// details from turning into mirror-like speckles. Order: black, white.
const MARBLE_ROUGHNESS_FACTORS: [f32; 2] = [1.4, 1.6];
const MARBLE_REFLECTION_STRENGTH: f32 = 0.72;
// Maximum planar-reflection blur at perceptual roughness 1.0, measured in
// reflection-render-target pixels. The shader scales it by roughness squared.
const MARBLE_REFLECTION_MAX_BLUR_PIXELS: f32 = 20.0;
pub(crate) const SQUARE_SIZE: f32 = 1.0;
const SQUARE_HEIGHT: f32 = 0.12;
const BOARD_RAIL_WIDTH: f32 = 0.4;
const BOARD_RAIL_HEIGHT: f32 = 0.16;
const BOARD_BASE_HEIGHT: f32 = 0.18;
const BOARD_BASE_CENTER_Y: f32 = -0.2;
pub(crate) const BOARD_BASE_BOTTOM_Y: f32 = BOARD_BASE_CENTER_Y - BOARD_BASE_HEIGHT * 0.5;
const HIGHLIGHT_PLANE_SIZE: f32 = SQUARE_SIZE;
// Highlights are exactly coplanar with the marble. Render depth bias keeps
// them on top without the parallax seam caused by a physically raised plane.
const HIGHLIGHT_DEPTH_BIAS: f32 = 2.0;
const HIGHLIGHT_MASK_SIZE: u32 = 128;
const SMALL_HIGHLIGHT_RADIUS: f32 = 0.13;
const HIGHLIGHT_FEATHER: f32 = 0.1;
const HIGHLIGHT_HALO_ALPHA: f32 = 0.14;
const LEGAL_HIGHLIGHT_COLOR: Color = Color::srgb(0.98, 0.08, 0.46);
const LEGAL_HOVER_COLOR: Color = Color::srgb(1.0, 0.4, 0.7);
const CAPTURE_HIGHLIGHT_COLOR: Color = Color::srgb(1.0, 0.015, 0.01);
const CAPTURE_HOVER_COLOR: Color = Color::srgb(1.0, 0.28, 0.2);
// Square highlight tuning. Distances are measured from the cell center: 0.0 is
// the center and 0.5 is an edge. The straight perimeter stays subtle, while
// both axes must approach an edge before the denser corner accent appears.
const SQUARE_HIGHLIGHT_FADE_START: f32 = 0.24;
const SQUARE_HIGHLIGHT_CORNER_START: f32 = 0.1;
const SQUARE_HIGHLIGHT_EDGE_OPACITY: f32 = 0.8;
const SQUARE_HIGHLIGHT_CORNER_OPACITY: f32 = 1.0;
const WOOD_ROUGHNESS_FACTOR: f32 = 1.45;
const WOOD_TEXTURE_WORLD_SIZE: f32 = 3.0;
const CAMERA_RADIUS_BOARD_SCALE: f32 = 1.42;
// Bevy picking considers press + release on the same entity a click even if a
// drag happened between them. Suppress move selection once camera orbiting has
// travelled farther than this screen-space distance.
const ORBIT_CLICK_CANCEL_DISTANCE: f32 = 4.0;

pub(crate) struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BoardRenderState>()
            .init_resource::<BoardPointerState>()
            .add_systems(Startup, setup_board_assets.after(PlanarReflectionStartup))
            .add_systems(Update, sync_board_geometry.in_set(FrontendSet::BoardSync))
            .add_systems(
                Update,
                update_square_highlights.in_set(FrontendSet::Highlights),
            );
    }
}

#[derive(Resource)]
struct BoardAssets {
    square_materials: [Handle<PlanarBoardMaterial>; 2],
    highlight_materials: HighlightMaterials,
    wood_material: Handle<StandardMaterial>,
}

struct HighlightMaterials {
    selected: [Handle<StandardMaterial>; 2],
    legal: [Handle<StandardMaterial>; 2],
    legal_hover: [Handle<StandardMaterial>; 2],
    capture: [Handle<StandardMaterial>; 2],
    capture_hover: [Handle<StandardMaterial>; 2],
    last: Handle<StandardMaterial>,
    check: [Handle<StandardMaterial>; 2],
}

#[derive(Resource, Default)]
struct BoardRenderState {
    size: Option<BoardSize>,
}

#[derive(Resource, Default)]
struct BoardPointerState {
    hovered_square: Option<Square>,
    primary_dragged: bool,
}

impl BoardPointerState {
    fn enter(&mut self, square: Square) {
        self.hovered_square = Some(square);
    }

    fn leave(&mut self, square: Square) {
        if self.hovered_square == Some(square) {
            self.hovered_square = None;
        }
    }

    fn begin_primary_press(&mut self) {
        self.primary_dragged = false;
    }

    fn record_primary_drag(&mut self, distance: Vec2) {
        if distance.length_squared() >= ORBIT_CLICK_CANCEL_DISTANCE.powi(2) {
            self.primary_dragged = true;
        }
    }
}

#[derive(Component, Clone, Copy)]
struct BoardSquare(Square);

#[derive(Component, Clone, Copy)]
struct BoardHighlight(Square);

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
    reflection_image: Res<PlanarReflectionImage>,
    mut marble_materials: ResMut<Assets<PlanarBoardMaterial>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let white_marble = load_filtered_texture(
        &asset_server,
        generated_surface_texture_path("white_marble_color"),
        true,
        false,
    );
    let black_marble = load_filtered_texture(
        &asset_server,
        generated_surface_texture_path("black_marble_color"),
        true,
        false,
    );
    let marble_textures = [&black_marble, &white_marble];
    let white_marble_normal = load_linear_texture(
        &asset_server,
        generated_surface_texture_path("white_marble_normalgl"),
    );
    let black_marble_normal = load_linear_texture(
        &asset_server,
        generated_surface_texture_path("black_marble_normalgl"),
    );
    let marble_normals = [&black_marble_normal, &white_marble_normal];
    let white_marble_roughness = load_linear_texture(
        &asset_server,
        generated_surface_texture_path("white_marble_roughness"),
    );
    let black_marble_roughness = load_linear_texture(
        &asset_server,
        generated_surface_texture_path("black_marble_roughness"),
    );
    let marble_roughness = [&black_marble_roughness, &white_marble_roughness];
    let wood_color = load_repeating_texture(
        &asset_server,
        generated_surface_texture_path("wood_color"),
        true,
    );
    let wood_normal = load_repeating_texture(
        &asset_server,
        generated_surface_texture_path("wood_normalgl"),
        false,
    );
    let wood_roughness = load_repeating_texture(
        &asset_server,
        generated_surface_texture_path("wood_roughness"),
        false,
    );

    let square_materials = marble_material_pair(
        &mut marble_materials,
        &reflection_image.0,
        marble_textures,
        marble_normals,
        marble_roughness,
    );
    // Empty destinations use a compact circle. Occupied squares and the last
    // move use a square edge gradient that leaves the piece itself unobscured.
    let highlight_masks = [
        images.add(radial_highlight_mask(SMALL_HIGHLIGHT_RADIUS)),
        images.add(square_edge_highlight_mask()),
    ];
    let highlight_materials = HighlightMaterials {
        selected: masked_highlight_materials(
            &mut standard_materials,
            &highlight_masks,
            Color::srgb(1.0, 0.48, 0.01),
        ),
        legal: masked_highlight_materials(
            &mut standard_materials,
            &highlight_masks,
            LEGAL_HIGHLIGHT_COLOR,
        ),
        legal_hover: masked_highlight_materials(
            &mut standard_materials,
            &highlight_masks,
            LEGAL_HOVER_COLOR,
        ),
        capture: masked_highlight_materials(
            &mut standard_materials,
            &highlight_masks,
            CAPTURE_HIGHLIGHT_COLOR,
        ),
        capture_hover: masked_highlight_materials(
            &mut standard_materials,
            &highlight_masks,
            CAPTURE_HOVER_COLOR,
        ),
        last: standard_materials.add(highlight_material(
            Color::srgb(0.025, 0.3, 1.0),
            Some(highlight_masks[1].clone()),
        )),
        check: masked_highlight_materials(
            &mut standard_materials,
            &highlight_masks,
            Color::srgb(1.0, 0.0, 0.015),
        ),
    };

    commands.insert_resource(BoardAssets {
        square_materials,
        highlight_materials,
        wood_material: standard_materials.add(StandardMaterial {
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
    materials: &mut Assets<PlanarBoardMaterial>,
    reflection_image: &Handle<Image>,
    color_textures: [&Handle<Image>; 2],
    normal_maps: [&Handle<Image>; 2],
    roughness_maps: [&Handle<Image>; 2],
) -> [Handle<PlanarBoardMaterial>; 2] {
    std::array::from_fn(|index| {
        materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(color_textures[index].clone()),
                normal_map_texture: Some(normal_maps[index].clone()),
                metallic_roughness_texture: Some(roughness_maps[index].clone()),
                emissive_texture: Some(reflection_image.clone()),
                metallic: 0.0,
                perceptual_roughness: MARBLE_ROUGHNESS_FACTORS[index],
                reflectance: 0.45,
                clearcoat: 0.25,
                clearcoat_perceptual_roughness: 0.18,
                ..default()
            },
            extension: PlanarReflectionExtension::new(
                MARBLE_REFLECTION_STRENGTH,
                MARBLE_REFLECTION_MAX_BLUR_PIXELS,
            ),
        })
    })
}

fn masked_highlight_materials(
    materials: &mut Assets<StandardMaterial>,
    masks: &[Handle<Image>; 2],
    color: Color,
) -> [Handle<StandardMaterial>; 2] {
    std::array::from_fn(|index| {
        materials.add(highlight_material(color, Some(masks[index].clone())))
    })
}

fn highlight_material(color: Color, mask: Option<Handle<Image>>) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        base_color_texture: mask,
        alpha_mode: AlphaMode::Blend,
        depth_bias: HIGHLIGHT_DEPTH_BIAS,
        unlit: true,
        fog_enabled: false,
        ..default()
    }
}

fn radial_highlight_mask(core_radius: f32) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: HIGHLIGHT_MASK_SIZE,
            height: HIGHLIGHT_MASK_SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[u8::MAX; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let size = HIGHLIGHT_MASK_SIZE as f32;
    let pixels = image
        .data
        .as_mut()
        .expect("a newly filled highlight image has pixel data");
    for y in 0..HIGHLIGHT_MASK_SIZE {
        for x in 0..HIGHLIGHT_MASK_SIZE {
            let uv = Vec2::new((x as f32 + 0.5) / size, (y as f32 + 0.5) / size);
            let distance = (uv - Vec2::splat(0.5)).length();
            let alpha = radial_highlight_alpha(distance, core_radius);
            let offset = ((y * HIGHLIGHT_MASK_SIZE + x) * 4 + 3) as usize;
            pixels[offset] = (alpha * f32::from(u8::MAX)).round() as u8;
        }
    }
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

fn radial_highlight_alpha(distance: f32, core_radius: f32) -> f32 {
    let feather_progress = ((distance - core_radius) / HIGHLIGHT_FEATHER).clamp(0.0, 1.0);
    let smooth = feather_progress * feather_progress * (3.0 - 2.0 * feather_progress);
    1.0 - smooth * (1.0 - HIGHLIGHT_HALO_ALPHA)
}

fn square_edge_highlight_mask() -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: HIGHLIGHT_MASK_SIZE,
            height: HIGHLIGHT_MASK_SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[u8::MAX; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let size = HIGHLIGHT_MASK_SIZE as f32;
    let pixels = image
        .data
        .as_mut()
        .expect("a newly filled highlight image has pixel data");
    for y in 0..HIGHLIGHT_MASK_SIZE {
        for x in 0..HIGHLIGHT_MASK_SIZE {
            let uv = Vec2::new((x as f32 + 0.5) / size, (y as f32 + 0.5) / size);
            let centered = (uv - Vec2::splat(0.5)).abs();
            let alpha = square_edge_highlight_alpha(centered);
            let offset = ((y * HIGHLIGHT_MASK_SIZE + x) * 4 + 3) as usize;
            pixels[offset] = (alpha * f32::from(u8::MAX)).round() as u8;
        }
    }
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

fn square_edge_highlight_alpha(centered: Vec2) -> f32 {
    let edge_fade = smoothstep(SQUARE_HIGHLIGHT_FADE_START, 0.5, centered.max_element());
    let corner = smoothstep(SQUARE_HIGHLIGHT_CORNER_START, 0.5, centered.x)
        * smoothstep(SQUARE_HIGHLIGHT_CORNER_START, 0.5, centered.y);
    let opacity = SQUARE_HIGHLIGHT_EDGE_OPACITY
        + (SQUARE_HIGHLIGHT_CORNER_OPACITY - SQUARE_HIGHLIGHT_EDGE_OPACITY) * corner;
    edge_fade * opacity
}

fn smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let progress = ((value - start) / (end - start)).clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
}

fn load_linear_texture(asset_server: &AssetServer, path: String) -> Handle<Image> {
    load_filtered_texture(asset_server, path, false, false)
}

fn load_repeating_texture(
    asset_server: &AssetServer,
    path: String,
    is_srgb: bool,
) -> Handle<Image> {
    load_filtered_texture(asset_server, path, is_srgb, true)
}

fn load_filtered_texture(
    asset_server: &AssetServer,
    path: String,
    is_srgb: bool,
    repeat: bool,
) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(move |settings: &mut ImageLoaderSettings| {
            settings.is_srgb = is_srgb;
            let mut sampler = ImageSamplerDescriptor::linear();
            sampler.anisotropy_clamp = TEXTURE_ANISOTROPY;
            if repeat {
                sampler.address_mode_u = ImageAddressMode::Repeat;
                sampler.address_mode_v = ImageAddressMode::Repeat;
            }
            settings.sampler = ImageSampler::Descriptor(sampler);
        })
        .load(path)
}

fn sync_board_geometry(
    mut commands: Commands,
    chess_match: Res<ChessMatch>,
    assets: Res<BoardAssets>,
    mut rendered: ResMut<BoardRenderState>,
    mut pointer: ResMut<BoardPointerState>,
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
    pointer.hovered_square = None;

    if let Ok(mut camera) = scene.cameras.single_mut() {
        let radius = board_camera_radius(size);
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
    let board_size = board_world_size(size);
    let width = board_size.x;
    let depth = board_size.y;
    let outer_width = width + BOARD_RAIL_WIDTH * 2.0;
    let outer_depth = depth + BOARD_RAIL_WIDTH * 2.0;
    let highlight_mesh = meshes.add(
        Plane3d::default()
            .mesh()
            .size(HIGHLIGHT_PLANE_SIZE, HIGHLIGHT_PLANE_SIZE),
    );

    let base_mesh = meshes.add(wood_mesh(Vec3::new(
        outer_width,
        BOARD_BASE_HEIGHT,
        outer_depth,
    )));
    spawn_wood_part(
        commands,
        assets,
        base_mesh,
        Transform::from_xyz(0.0, BOARD_BASE_CENTER_Y, 0.0),
    );

    let horizontal_rail = meshes.add(wood_mesh(Vec3::new(
        outer_width,
        BOARD_RAIL_HEIGHT,
        BOARD_RAIL_WIDTH,
    )));
    let horizontal_rail_offset = depth * 0.5 + BOARD_RAIL_WIDTH * 0.5;
    for z in [-horizontal_rail_offset, horizontal_rail_offset] {
        spawn_wood_part(
            commands,
            assets,
            horizontal_rail.clone(),
            Transform::from_xyz(0.0, -0.04, z),
        );
    }

    let vertical_rail = meshes.add(wood_mesh(Vec3::new(
        depth,
        BOARD_RAIL_HEIGHT,
        BOARD_RAIL_WIDTH,
    )));
    let vertical_rail_offset = width * 0.5 + BOARD_RAIL_WIDTH * 0.5;
    for x in [-vertical_rail_offset, vertical_rail_offset] {
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
            let material = assets.square_materials[material_index(square)].clone();
            commands
                .spawn((
                    Mesh3d(meshes.add(square_mesh(square, size))),
                    MeshMaterial3d(material),
                    Transform::from_translation(
                        square_world(square, size) - Vec3::Y * (SQUARE_HEIGHT * 0.5),
                    ),
                    NotShadowCaster,
                    BoardSquare(square),
                ))
                .observe(on_square_press)
                .observe(on_square_drag)
                .observe(on_square_click)
                .observe(on_square_over)
                .observe(on_square_out);
            commands.spawn((
                Mesh3d(highlight_mesh.clone()),
                MeshMaterial3d(assets.highlight_materials.legal[0].clone()),
                Transform::from_translation(square_world(square, size)),
                Visibility::Hidden,
                Pickable::IGNORE,
                NotShadowCaster,
                BoardHighlight(square),
                BoardPart,
            ));
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
        NotShadowCaster,
        BoardPart,
    ));
}

fn square_mesh(square: Square, size: BoardSize) -> Mesh {
    let mut mesh = Mesh::from(Cuboid::new(SQUARE_SIZE, SQUARE_HEIGHT, SQUARE_SIZE));
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
    menu: Res<GameMenuState>,
    animation: Res<PieceAnimationState>,
    pointer: Res<BoardPointerState>,
    mut chess_match: ResMut<ChessMatch>,
    mut move_requests: MessageWriter<MoveRequest>,
) {
    if menu.open
        || !animation.is_settled(chess_match.generation)
        || click.button != PointerButton::Primary
        || pointer.primary_dragged
    {
        return;
    }
    let Ok(clicked) = squares.get(click.entity) else {
        return;
    };
    if let Some(chess_move) = handle_square_selection(&mut chess_match, clicked.0) {
        move_requests.write(MoveRequest(chess_move));
    }
}

fn on_square_press(press: On<Pointer<Press>>, mut pointer: ResMut<BoardPointerState>) {
    if press.button == PointerButton::Primary {
        pointer.begin_primary_press();
    }
}

fn on_square_drag(drag: On<Pointer<Drag>>, mut pointer: ResMut<BoardPointerState>) {
    if drag.button == PointerButton::Primary {
        pointer.record_primary_drag(drag.distance);
    }
}

fn on_square_over(
    over: On<Pointer<Over>>,
    squares: Query<&BoardSquare>,
    mut pointer: ResMut<BoardPointerState>,
) {
    if let Ok(square) = squares.get(over.entity) {
        pointer.enter(square.0);
    }
}

fn on_square_out(
    out: On<Pointer<Out>>,
    squares: Query<&BoardSquare>,
    mut pointer: ResMut<BoardPointerState>,
) {
    let Ok(square) = squares.get(out.entity) else {
        return;
    };
    pointer.leave(square.0);
}

fn update_square_highlights(
    chess_match: Res<ChessMatch>,
    menu: Res<GameMenuState>,
    pointer: Res<BoardPointerState>,
    assets: Res<BoardAssets>,
    mut highlights: Query<(
        &BoardHighlight,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
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

    for (board_highlight, mut material, mut visibility) in &mut highlights {
        let square = board_highlight.0;
        let occupied_index = usize::from(position.board().piece_at(square).is_some());
        let highlighted_material = if checked_king == Some(square) {
            Some(&assets.highlight_materials.check[occupied_index])
        } else if chess_match.selected == Some(square) {
            Some(&assets.highlight_materials.selected[occupied_index])
        } else if let Some(chess_move) = selected_moves
            .iter()
            .find(|chess_move| chess_move.to == square)
        {
            let hovered = !menu.open && pointer.hovered_square == Some(square);
            let capture = matches!(chess_move.kind, MoveKind::EnPassant)
                || position.board().piece_at(square).is_some();
            if capture && hovered {
                Some(&assets.highlight_materials.capture_hover[occupied_index])
            } else if capture {
                Some(&assets.highlight_materials.capture[occupied_index])
            } else if hovered {
                Some(&assets.highlight_materials.legal_hover[occupied_index])
            } else {
                Some(&assets.highlight_materials.legal[occupied_index])
            }
        } else if chess_match
            .last_move
            .is_some_and(|last| last.from == square || last.to == square)
        {
            Some(&assets.highlight_materials.last)
        } else {
            None
        };
        if let Some(highlighted_material) = highlighted_material {
            material.0 = highlighted_material.clone();
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}

fn material_index(square: Square) -> usize {
    usize::from(!(square.file() + square.rank()).is_multiple_of(2))
}

pub(crate) fn square_world(square: Square, size: BoardSize) -> Vec3 {
    Vec3::new(
        (f32::from(square.file()) - (f32::from(size.files()) - 1.0) * 0.5) * SQUARE_SIZE,
        0.0,
        (f32::from(square.rank()) - (f32::from(size.ranks()) - 1.0) * 0.5) * SQUARE_SIZE,
    )
}

pub(crate) fn board_world_size(size: BoardSize) -> Vec2 {
    Vec2::new(
        f32::from(size.files()) * SQUARE_SIZE,
        f32::from(size.ranks()) * SQUARE_SIZE,
    )
}

pub(crate) fn board_camera_radius(size: BoardSize) -> f32 {
    board_world_size(size).max_element() * CAMERA_RADIUS_BOARD_SCALE
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
    fn adjacent_squares_and_highlights_meet_exactly_at_their_edges() {
        for size in [BoardSize::CAPABLANCA, BoardSize::GRAND] {
            let lower = square_world(Square::new(0, 0), size);
            let right = square_world(Square::new(1, 0), size);
            let forward = square_world(Square::new(0, 1), size);
            let world_size = board_world_size(size);

            assert_eq!(right.x - lower.x, SQUARE_SIZE);
            assert_eq!(forward.z - lower.z, SQUARE_SIZE);
            assert_eq!(lower.x - SQUARE_SIZE * 0.5, -world_size.x * 0.5);
            assert_eq!(lower.z - SQUARE_SIZE * 0.5, -world_size.y * 0.5);
            assert_eq!(HIGHLIGHT_PLANE_SIZE, SQUARE_SIZE);
        }
    }

    #[test]
    fn square_mesh_fills_the_complete_cell_footprint() {
        let mesh = square_mesh(Square::new(0, 0), BoardSize::CAPABLANCA);
        let VertexAttributeValues::Float32x3(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("a square mesh has vertex positions")
        else {
            panic!("square mesh positions use an unexpected format");
        };
        let (minimum, maximum) = positions.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(minimum, maximum), position| {
                let position = Vec3::from_array(*position);
                (minimum.min(position), maximum.max(position))
            },
        );
        let extent = maximum - minimum;

        assert_eq!(extent.x, SQUARE_SIZE);
        assert_eq!(extent.y, SQUARE_HEIGHT);
        assert_eq!(extent.z, SQUARE_SIZE);
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

    #[test]
    fn highlights_use_a_surface_independent_unlit_overlay() {
        let material = highlight_material(Color::srgba(0.1, 0.8, 0.2, 0.5), None);
        assert!(material.unlit);
        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        assert!(material.depth_bias > 0.0);
        assert!(!material.fog_enabled);
    }

    #[test]
    fn radial_highlight_has_an_opaque_core_and_transparent_cell_halo() {
        assert_eq!(radial_highlight_alpha(0.0, SMALL_HIGHLIGHT_RADIUS), 1.0);
        assert!(
            (radial_highlight_alpha(1.0, SMALL_HIGHLIGHT_RADIUS) - HIGHLIGHT_HALO_ALPHA).abs()
                < f32::EPSILON
        );
        let feather = radial_highlight_alpha(
            SMALL_HIGHLIGHT_RADIUS + HIGHLIGHT_FEATHER * 0.5,
            SMALL_HIGHLIGHT_RADIUS,
        );
        assert!(feather > HIGHLIGHT_HALO_ALPHA && feather < 1.0);
    }

    #[test]
    fn occupied_highlight_is_subtle_at_edges_and_dense_only_in_corners() {
        assert_eq!(square_edge_highlight_alpha(Vec2::ZERO), 0.0);
        assert_eq!(
            square_edge_highlight_alpha(Vec2::new(0.5, 0.0)),
            SQUARE_HIGHLIGHT_EDGE_OPACITY
        );
        assert_eq!(
            square_edge_highlight_alpha(Vec2::splat(0.5)),
            SQUARE_HIGHLIGHT_CORNER_OPACITY
        );

        let inner_corner = square_edge_highlight_alpha(Vec2::splat(0.4));
        let farther_from_corner = square_edge_highlight_alpha(Vec2::splat(0.3));
        assert!(inner_corner > farther_from_corner);
        assert!(inner_corner < SQUARE_HIGHLIGHT_CORNER_OPACITY);
    }

    #[test]
    fn leaving_an_old_square_does_not_clear_the_new_hover() {
        let first = Square::new(2, 2);
        let second = Square::new(3, 2);
        let mut pointer = BoardPointerState::default();

        pointer.enter(first);
        pointer.enter(second);
        pointer.leave(first);
        assert_eq!(pointer.hovered_square, Some(second));

        pointer.leave(second);
        assert_eq!(pointer.hovered_square, None);
    }

    #[test]
    fn camera_drag_cancels_a_square_click_only_after_the_motion_threshold() {
        let mut pointer = BoardPointerState::default();
        pointer.begin_primary_press();
        pointer.record_primary_drag(Vec2::new(3.0, 2.0));
        assert!(!pointer.primary_dragged);

        pointer.record_primary_drag(Vec2::new(4.0, 0.0));
        assert!(pointer.primary_dragged);

        pointer.begin_primary_press();
        assert!(!pointer.primary_dragged);
    }
}
