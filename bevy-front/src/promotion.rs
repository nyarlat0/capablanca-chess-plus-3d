use std::f32::consts::PI;

use bevy::{
    camera::{RenderTarget, primitives::Aabb, visibility::RenderLayers},
    ecs::system::SystemParam,
    prelude::*,
    render::render_resource::TextureFormat,
    window::PrimaryWindow,
};
use capablanca_chess_plus::{Color as Side, Move, PieceKind};

use crate::{
    app::FrontendSet,
    game::{ChessMatch, MoveRequest},
    menu::GameMenuState,
    pieces::{PieceAssets, PieceMaterial, piece_model_scale},
};

const ACCENT: Color = Color::srgb(0.98, 0.19, 0.52);
const ACCENT_HOVER: Color = Color::srgb(1.0, 0.31, 0.62);
const PANEL: Color = Color::srgba(0.035, 0.025, 0.055, 0.94);
const PREVIEW_TEXTURE_SIZE: u32 = 256;
const PREVIEW_LAYER_BASE: usize = 8;
const PREVIEW_ROTATION_SPEED: f32 = 0.7;
const PREVIEW_MODEL_SCALE: f32 = 1.7;
const PREVIEW_TILT_RADIANS: f32 = PI / 6.0;
const PREVIEW_LIGHT_ILLUMINANCE: f32 = 11_000.0;
const WEB_PREVIEW_LIGHT_INTENSITY: f32 = 2_400_000.0;
const WEB_PREVIEW_LIGHT_RANGE: f32 = 10.0;
const WEB_PREVIEW_LIGHT_RADIUS: f32 = 0.8;

pub(crate) struct PromotionPlugin;

impl Plugin for PromotionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PromotionUiState>()
            .add_systems(Update, handle_promotion_choice.in_set(FrontendSet::Input))
            .add_systems(
                Update,
                (
                    sync_promotion_popup,
                    center_promotion_previews,
                    rotate_promotion_previews,
                    adapt_promotion_layout,
                    style_promotion_choices,
                )
                    .chain()
                    .in_set(FrontendSet::Hud),
            );
    }
}

#[derive(Resource, Default)]
struct PromotionUiState {
    options: Vec<Option<PieceKind>>,
    images: Vec<Handle<Image>>,
    retired_images: Vec<Handle<Image>>,
}

#[derive(Component)]
struct PromotionPopupRoot;

#[derive(Component)]
struct PromotionPanel;

#[derive(Component)]
struct PromotionGrid;

#[derive(Component)]
struct PromotionChoiceImage;

#[derive(Component)]
struct PromotionTextSize(f32);

#[derive(Component)]
struct PromotionPreviewEntity;

#[derive(Component)]
struct PromotionPreviewPiece;

#[derive(Component)]
struct PromotionPreviewModel {
    pivot: Entity,
    bounds_observed: bool,
}

#[derive(Component)]
struct PromotionPreviewCentered;

#[derive(Component)]
struct PromotionPreviewReady;

#[derive(Component, Clone, Copy)]
struct PromotionChoice(Option<PieceKind>);

#[derive(Component)]
struct PromotionChoiceLabel;

#[derive(SystemParam)]
struct PromotionPopupParams<'w, 's> {
    commands: Commands<'w, 's>,
    asset_server: Res<'w, AssetServer>,
    menu: Res<'w, GameMenuState>,
    chess_match: Res<'w, ChessMatch>,
    piece_assets: Res<'w, PieceAssets>,
    images: ResMut<'w, Assets<Image>>,
    state: ResMut<'w, PromotionUiState>,
    popup_roots: Query<'w, 's, Entity, With<PromotionPopupRoot>>,
    preview_entities: Query<'w, 's, Entity, With<PromotionPreviewEntity>>,
}

fn handle_promotion_choice(
    choices: Query<(&Interaction, &PromotionChoice), Changed<Interaction>>,
    chess_match: Res<ChessMatch>,
    mut move_requests: MessageWriter<MoveRequest>,
) {
    let Some(choice) = choices.iter().find_map(|(interaction, choice)| {
        (*interaction == Interaction::Pressed).then_some(choice.0)
    }) else {
        return;
    };
    let chess_move = chess_match.pending_promotion.as_ref().and_then(|pending| {
        pending
            .moves
            .iter()
            .copied()
            .find(|candidate| candidate.promotion == choice)
    });
    if let Some(chess_move) = chess_move {
        move_requests.write(MoveRequest(chess_move));
    }
}

fn sync_promotion_popup(params: PromotionPopupParams) {
    let PromotionPopupParams {
        mut commands,
        asset_server,
        menu,
        chess_match,
        piece_assets,
        mut images,
        mut state,
        popup_roots,
        preview_entities,
    } = params;
    for image in state.retired_images.drain(..) {
        images.remove(image.id());
    }

    let options = if menu.open {
        Vec::new()
    } else {
        chess_match
            .pending_promotion
            .as_ref()
            .map_or_else(Vec::new, |pending| promotion_options(&pending.moves))
    };
    if options == state.options {
        return;
    }

    for entity in &popup_roots {
        commands.entity(entity).despawn();
    }
    for entity in &preview_entities {
        commands.entity(entity).despawn();
    }
    let old_images = std::mem::take(&mut state.images);
    state.retired_images.extend(old_images);
    state.options = options.clone();
    if options.is_empty() {
        return;
    }

    let side = chess_match.game.position().side_to_move();
    let mut previews = Vec::with_capacity(options.len());
    for (index, promotion) in options.iter().copied().enumerate() {
        let image = images.add(Image::new_target_texture(
            PREVIEW_TEXTURE_SIZE,
            PREVIEW_TEXTURE_SIZE,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        spawn_promotion_preview(
            &mut commands,
            &piece_assets,
            side,
            promotion.unwrap_or(PieceKind::Pawn),
            PREVIEW_LAYER_BASE + index,
            image.clone(),
        );
        state.images.push(image.clone());
        previews.push((promotion, image));
    }

    let font: Handle<Font> = asset_server.load("fonts/FiraSans-Bold.ttf");
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                padding: UiRect::all(px(18)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.008, 0.02, 0.62)),
            GlobalZIndex(90),
            PromotionPopupRoot,
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: percent(94),
                        max_width: px(1_080),
                        max_height: percent(98),
                        padding: UiRect::axes(px(24), px(22)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(24)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: px(8),
                        overflow: Overflow::clip_y(),
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    BorderColor::all(Color::srgba(1.0, 0.38, 0.65, 0.42)),
                    BoxShadow::new(
                        Color::srgba(0.0, 0.0, 0.0, 0.72),
                        px(0),
                        px(14),
                        px(5),
                        px(36),
                    ),
                    PromotionPanel,
                ))
                .with_children(|panel| {
                    panel.spawn(popup_text(&font, "CHOOSE PROMOTION", 24.0, ACCENT));
                    panel.spawn(popup_text(
                        &font,
                        "Select one of the available pieces",
                        13.0,
                        Color::srgb(0.67, 0.64, 0.72),
                    ));
                    panel
                        .spawn((
                            Node {
                                width: percent(100),
                                margin: UiRect::top(px(10)),
                                flex_wrap: FlexWrap::Wrap,
                                justify_content: JustifyContent::Center,
                                column_gap: px(10),
                                row_gap: px(10),
                                ..default()
                            },
                            PromotionGrid,
                        ))
                        .with_children(|grid| {
                            for (promotion, image) in previews {
                                spawn_promotion_choice(grid, &font, promotion, image);
                            }
                        });
                });
        });
}

fn spawn_promotion_preview(
    commands: &mut Commands,
    assets: &PieceAssets,
    side: Side,
    kind: PieceKind,
    layer: usize,
    image: Handle<Image>,
) {
    let render_layer = RenderLayers::layer(layer);
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -10,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(image.into()),
        Transform::from_xyz(0.0, 1.35, 4.1).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
        render_layer.clone(),
        PromotionPreviewEntity,
    ));
    if cfg!(target_arch = "wasm32") {
        // WebGL2 supports only one directional light, which is reserved for
        // the board. A local point light gives every isolated preview the same
        // readable shape without consuming that slot.
        commands.spawn((
            PointLight {
                intensity: WEB_PREVIEW_LIGHT_INTENSITY,
                range: WEB_PREVIEW_LIGHT_RANGE,
                radius: WEB_PREVIEW_LIGHT_RADIUS,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(-2.0, 3.0, 3.0),
            render_layer,
            PromotionPreviewEntity,
        ));
    } else {
        commands.spawn((
            DirectionalLight {
                illuminance: PREVIEW_LIGHT_ILLUMINANCE,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.75, -0.55, 0.0)),
            render_layer,
            PromotionPreviewEntity,
        ));
    }
    let pivot = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Visible,
            PromotionPreviewPiece,
            PromotionPreviewEntity,
        ))
        .id();
    commands.entity(pivot).with_child((
        WorldAssetRoot(assets.scene(kind)),
        Transform::from_rotation(preview_model_rotation(side))
            .with_scale(Vec3::splat(piece_model_scale() * PREVIEW_MODEL_SCALE)),
        PieceMaterial::preview(assets.material(side), layer),
        PromotionPreviewModel {
            pivot,
            bounds_observed: false,
        },
    ));
}

fn preview_model_rotation(side: Side) -> Quat {
    Quat::from_rotation_x(PREVIEW_TILT_RADIANS)
        * Quat::from_rotation_y(if side == Side::Black { PI } else { 0.0 })
}

fn spawn_promotion_choice(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    promotion: Option<PieceKind>,
    image: Handle<Image>,
) {
    parent
        .spawn((
            Button,
            Node {
                width: px(136),
                height: px(174),
                padding: UiRect::all(px(7)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(16)),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.1, 0.075, 0.14, 0.82)),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.12)),
            PromotionChoice(promotion),
        ))
        .with_children(|button| {
            button.spawn((
                ImageNode::new(image),
                Node {
                    width: px(122),
                    height: px(132),
                    ..default()
                },
                Pickable::IGNORE,
                PromotionChoiceImage,
            ));
            button.spawn((
                popup_text(
                    font,
                    promotion_label(promotion),
                    13.0,
                    Color::srgb(0.95, 0.93, 0.97),
                ),
                PromotionChoiceLabel,
                Pickable::IGNORE,
            ));
        });
}

const fn promotion_label(promotion: Option<PieceKind>) -> &'static str {
    match promotion {
        None => "Keep pawn",
        Some(PieceKind::Pawn) => "Pawn",
        Some(PieceKind::Knight) => "Knight",
        Some(PieceKind::Bishop) => "Bishop",
        Some(PieceKind::Rook) => "Rook",
        Some(PieceKind::Queen) => "Queen",
        Some(PieceKind::King) => "King",
        Some(PieceKind::Archbishop) => "Archbishop",
        Some(PieceKind::Chancellor) => "Chancellor",
    }
}

fn popup_text(font: &Handle<Font>, value: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(value),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        PromotionTextSize(size),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PromotionLayout {
    Regular,
    Compact,
    Tiny,
}

fn promotion_layout_for_size(width: f32, height: f32) -> PromotionLayout {
    if width < 380.0 || height < 440.0 {
        PromotionLayout::Tiny
    } else if width < 640.0 || height < 680.0 {
        PromotionLayout::Compact
    } else {
        PromotionLayout::Regular
    }
}

fn adapt_promotion_layout(
    window: Single<&Window, With<PrimaryWindow>>,
    mut nodes: ParamSet<(
        Query<&mut Node, With<PromotionPopupRoot>>,
        Query<&mut Node, (With<PromotionPanel>, Without<PromotionPopupRoot>)>,
        Query<&mut Node, (With<PromotionGrid>, Without<PromotionPanel>)>,
        Query<&mut Node, (With<PromotionChoice>, Without<PromotionChoiceImage>)>,
        Query<&mut Node, With<PromotionChoiceImage>>,
    )>,
    mut text: Query<(&PromotionTextSize, &mut TextFont)>,
) {
    let layout = promotion_layout_for_size(window.width(), window.height());
    let (
        root_padding,
        panel_x,
        panel_y,
        panel_gap,
        grid_gap,
        card_width,
        card_height,
        image_width,
        image_height,
        text_scale,
    ) = match layout {
        PromotionLayout::Regular => (18.0, 24.0, 22.0, 8.0, 10.0, 136.0, 174.0, 122.0, 132.0, 1.0),
        PromotionLayout::Compact => (8.0, 10.0, 10.0, 6.0, 7.0, 96.0, 126.0, 82.0, 90.0, 0.8),
        PromotionLayout::Tiny => (4.0, 6.0, 5.0, 4.0, 6.0, 82.0, 105.0, 70.0, 74.0, 0.7),
    };

    for mut root in &mut nodes.p0() {
        root.padding = UiRect::all(px(root_padding));
    }
    for mut panel in &mut nodes.p1() {
        panel.width = percent(if layout == PromotionLayout::Regular {
            94
        } else {
            99
        });
        panel.padding = UiRect::axes(px(panel_x), px(panel_y));
        panel.row_gap = px(panel_gap);
    }
    for mut grid in &mut nodes.p2() {
        grid.margin = UiRect::top(px(if layout == PromotionLayout::Regular {
            10.0
        } else {
            3.0
        }));
        grid.column_gap = px(grid_gap);
        grid.row_gap = px(grid_gap);
    }
    for mut card in &mut nodes.p3() {
        card.width = px(card_width);
        card.height = px(card_height);
        card.padding = UiRect::all(px(if layout == PromotionLayout::Regular {
            7.0
        } else {
            4.0
        }));
    }
    for mut image in &mut nodes.p4() {
        image.width = px(image_width);
        image.height = px(image_height);
    }
    for (size, mut font) in &mut text {
        font.font_size = FontSize::Px(size.0 * text_scale);
    }
}

fn promotion_options(moves: &[Move]) -> Vec<Option<PieceKind>> {
    let mut options = Vec::new();
    for promotion in moves.iter().map(|chess_move| chess_move.promotion) {
        if !options.contains(&promotion) {
            options.push(promotion);
        }
    }
    options
}

fn center_promotion_previews(
    mut commands: Commands,
    mut models: Query<
        (Entity, &mut PromotionPreviewModel, &mut Transform),
        Without<PromotionPreviewCentered>,
    >,
    pivots: Query<&GlobalTransform, With<PromotionPreviewPiece>>,
    children: Query<&Children>,
    mesh_bounds: Query<(&Aabb, &GlobalTransform), With<Mesh3d>>,
) {
    for (model_entity, mut model, mut model_transform) in &mut models {
        let Ok(pivot_transform) = pivots.get(model.pivot) else {
            continue;
        };
        let pivot_from_world = pivot_transform.affine().inverse();
        let mut minimum = Vec3::splat(f32::INFINITY);
        let mut maximum = Vec3::splat(f32::NEG_INFINITY);
        let mut found_bounds = false;

        for descendant in children.iter_descendants(model_entity) {
            let Ok((aabb, mesh_transform)) = mesh_bounds.get(descendant) else {
                continue;
            };
            let mesh_to_pivot = pivot_from_world * mesh_transform.affine();
            let center = Vec3::from(aabb.center);
            let half_extents = Vec3::from(aabb.half_extents);
            for x in [-1.0, 1.0] {
                for y in [-1.0, 1.0] {
                    for z in [-1.0, 1.0] {
                        let corner = center + half_extents * Vec3::new(x, y, z);
                        let point = mesh_to_pivot.transform_point3(corner);
                        minimum = minimum.min(point);
                        maximum = maximum.max(point);
                    }
                }
            }
            found_bounds = true;
        }

        if !found_bounds {
            continue;
        }
        // AABBs can appear in the same frame as the GLB hierarchy. Wait for one
        // transform-propagation pass before using their GlobalTransforms.
        if !model.bounds_observed {
            model.bounds_observed = true;
            continue;
        }

        let bounds_center = (minimum + maximum) * 0.5;
        model_transform.translation += horizontal_axis_centering_offset(bounds_center);
        commands
            .entity(model_entity)
            .insert(PromotionPreviewCentered);
        commands.entity(model.pivot).insert(PromotionPreviewReady);
    }
}

fn horizontal_axis_centering_offset(center: Vec3) -> Vec3 {
    Vec3::new(-center.x, 0.0, -center.z)
}

fn rotate_promotion_previews(
    time: Res<Time>,
    mut previews: Query<&mut Transform, (With<PromotionPreviewPiece>, With<PromotionPreviewReady>)>,
) {
    for mut transform in &mut previews {
        // Bevy is Y-up: the parent spins around the height axis, while the
        // child keeps its fixed 30-degree lean away from that same axis.
        transform.rotate_y(PREVIEW_ROTATION_SPEED * time.delta_secs());
    }
}

fn style_promotion_choices(
    mut choices: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &Children,
        ),
        With<PromotionChoice>,
    >,
    mut labels: Query<&mut TextColor, With<PromotionChoiceLabel>>,
) {
    for (interaction, mut background, mut border, children) in &mut choices {
        let (background_color, border_color, label_color) = match interaction {
            Interaction::None => (
                Color::srgba(0.1, 0.075, 0.14, 0.82),
                Color::srgba(1.0, 1.0, 1.0, 0.12),
                Color::srgb(0.95, 0.93, 0.97),
            ),
            Interaction::Hovered => (
                Color::srgba(0.31, 0.09, 0.2, 0.9),
                ACCENT_HOVER,
                Color::WHITE,
            ),
            Interaction::Pressed => (
                Color::srgba(0.48, 0.07, 0.23, 0.96),
                Color::srgb(1.0, 0.72, 0.85),
                Color::WHITE,
            ),
        };
        background.0 = background_color;
        *border = BorderColor::all(border_color);
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child) {
                label.0 = label_color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use capablanca_chess_plus::Square;

    use super::*;

    #[test]
    fn promotion_options_are_unique_and_keep_grand_no_promotion() {
        let from = Square::new(0, 7);
        let to = Square::new(0, 8);
        let moves = [
            Move::normal(from, to),
            Move::promotion(from, to, PieceKind::Queen),
            Move::promotion(from, to, PieceKind::Queen),
            Move::promotion(from, to, PieceKind::Chancellor),
        ];

        assert_eq!(
            promotion_options(&moves),
            vec![None, Some(PieceKind::Queen), Some(PieceKind::Chancellor)]
        );
    }

    #[test]
    fn previews_lean_thirty_degrees_and_orbit_the_y_height_axis() {
        for side in Side::ALL {
            let tilted_height = preview_model_rotation(side) * Vec3::Y;
            assert!((tilted_height.angle_between(Vec3::Y) - PREVIEW_TILT_RADIANS).abs() < 0.000_01);

            let spun_height = Quat::from_rotation_y(1.1) * tilted_height;
            assert!((spun_height.y - tilted_height.y).abs() < 0.000_01);
            assert!((spun_height.length() - tilted_height.length()).abs() < 0.000_01);
        }
    }

    #[test]
    fn preview_bounds_center_is_placed_on_the_height_axis() {
        let center = Vec3::new(0.37, 1.42, -0.28);
        let centered = center + horizontal_axis_centering_offset(center);

        assert!(centered.x.abs() < f32::EPSILON);
        assert!((centered.y - center.y).abs() < f32::EPSILON);
        assert!(centered.z.abs() < f32::EPSILON);

        let spun = Quat::from_rotation_y(1.7) * centered;
        assert!(spun.x.abs() < f32::EPSILON);
        assert!(spun.z.abs() < f32::EPSILON);
    }
}
