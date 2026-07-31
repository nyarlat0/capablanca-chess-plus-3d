use bevy::{ecs::hierarchy::ChildSpawnerCommands, prelude::*};
use bevy_panorbit_camera::PanOrbitCamera;
use capablanca_chess_plus::{Color as Side, Variant};

use crate::{
    ai::AiTask,
    app::FrontendSet::Menu,
    game::{ChessMatch, Controller, restart_match},
    scene::{CameraAutoTurn, start_camera_turn},
};

const ACCENT: Color = Color::srgb(0.98, 0.19, 0.52);
const ACCENT_HOVER: Color = Color::srgb(1.0, 0.3, 0.61);
const TEXT_PRIMARY: Color = Color::srgb(0.97, 0.95, 0.98);
const TEXT_MUTED: Color = Color::srgb(0.65, 0.62, 0.7);
const PANEL: Color = Color::srgba(0.035, 0.025, 0.055, 0.86);
const BUTTON: Color = Color::srgba(0.12, 0.09, 0.16, 0.76);
const BUTTON_HOVER: Color = Color::srgba(0.31, 0.11, 0.22, 0.88);
const BUTTON_SELECTED: Color = Color::srgba(0.72, 0.08, 0.3, 0.72);
const MENU_ORBIT_SPEED: f32 = 0.09;

pub(crate) struct GameMenuPlugin;

impl Plugin for GameMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMenuState>()
            .add_systems(Startup, setup_game_menu)
            .add_systems(
                Update,
                (
                    handle_menu_interactions,
                    sync_menu_visibility,
                    style_menu_buttons,
                    animate_menu_background,
                )
                    .chain()
                    .in_set(Menu),
            );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GameMode {
    Local,
    Ai,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SideChoice {
    Random,
    White,
    Black,
}

#[derive(Resource)]
pub(crate) struct GameMenuState {
    pub(crate) open: bool,
    pub(crate) selected_mode: GameMode,
    pub(crate) selected_variant: Variant,
    pub(crate) selected_side: SideChoice,
    pub(crate) active_mode: GameMode,
    pub(crate) active_side: Side,
}

impl Default for GameMenuState {
    fn default() -> Self {
        Self {
            open: true,
            selected_mode: GameMode::Local,
            selected_variant: Variant::Gothic,
            selected_side: SideChoice::Random,
            active_mode: GameMode::Local,
            active_side: Side::White,
        }
    }
}

#[derive(Component)]
struct GameMenuRoot;

#[derive(Component, Clone, Copy)]
enum MenuAction {
    Mode(GameMode),
    Variant(Variant),
    Side(SideChoice),
    Multiplayer,
    Start,
}

#[derive(Component)]
struct MenuButtonLabel;

fn setup_game_menu(mut commands: Commands, asset_server: Res<AssetServer>) {
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
            BackgroundColor(Color::srgba(0.012, 0.008, 0.022, 0.38)),
            GlobalZIndex(100),
            GameMenuRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: percent(92),
                    max_width: px(780),
                    max_height: percent(96),
                    padding: UiRect::axes(px(34), px(28)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(24)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(17),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(Color::srgba(1.0, 0.38, 0.65, 0.3)),
                BoxShadow::new(
                    Color::srgba(0.0, 0.0, 0.0, 0.72),
                    px(0),
                    px(14),
                    px(5),
                    px(36),
                ),
            ))
            .with_children(|panel| {
                panel.spawn(text(&font, "CAPABLANCA CHESS +", 32.0, ACCENT));
                panel.spawn(text(
                    &font,
                    "Configure a game while the board waits behind the glass.",
                    14.0,
                    TEXT_MUTED,
                ));

                spawn_section_label(panel, &font, "GAME MODE");
                spawn_row(panel, |row| {
                    spawn_menu_button(
                        row,
                        &font,
                        "Local",
                        MenuAction::Mode(GameMode::Local),
                        percent(32),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "AI",
                        MenuAction::Mode(GameMode::Ai),
                        percent(32),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "Multiplayer · Soon",
                        MenuAction::Multiplayer,
                        percent(32),
                    );
                });

                spawn_section_label(panel, &font, "RULES & STARTING POSITION");
                panel
                    .spawn(Node {
                        width: percent(100),
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: px(8),
                        row_gap: px(8),
                        ..default()
                    })
                    .with_children(|grid| {
                        for variant in Variant::ALL {
                            spawn_menu_button(
                                grid,
                                &font,
                                variant_label(variant),
                                MenuAction::Variant(variant),
                                percent(24),
                            );
                        }
                    });

                spawn_section_label(panel, &font, "PLAY AS");
                spawn_row(panel, |row| {
                    spawn_menu_button(
                        row,
                        &font,
                        "Random",
                        MenuAction::Side(SideChoice::Random),
                        percent(32),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "White",
                        MenuAction::Side(SideChoice::White),
                        percent(32),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "Black",
                        MenuAction::Side(SideChoice::Black),
                        percent(32),
                    );
                });

                spawn_menu_button(panel, &font, "START GAME", MenuAction::Start, percent(100));
            });
        });
}

fn spawn_section_label(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, value: &str) {
    parent.spawn((
        text(font, value, 12.0, Color::srgba(1.0, 0.48, 0.7, 0.9)),
        Node {
            margin: UiRect::top(px(3)),
            ..default()
        },
    ));
}

fn spawn_row(parent: &mut ChildSpawnerCommands, children: impl FnOnce(&mut ChildSpawnerCommands)) {
    parent
        .spawn(Node {
            width: percent(100),
            column_gap: px(8),
            ..default()
        })
        .with_children(children);
}

fn spawn_menu_button(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    label: &str,
    action: MenuAction,
    width: Val,
) {
    let is_start = matches!(action, MenuAction::Start);
    parent
        .spawn((
            Button,
            Node {
                width,
                min_width: px(120),
                height: px(if is_start { 54 } else { 44 }),
                flex_grow: 1.0,
                padding: UiRect::axes(px(12), px(8)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(if is_start { 15 } else { 12 })),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(if is_start { ACCENT } else { BUTTON }),
            BorderColor::all(if is_start {
                Color::srgba(1.0, 0.68, 0.82, 0.8)
            } else {
                Color::srgba(1.0, 1.0, 1.0, 0.1)
            }),
            action,
        ))
        .with_child((text(font, label, 14.0, TEXT_PRIMARY), MenuButtonLabel));
}

fn text(font: &Handle<Font>, value: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(value),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn handle_menu_interactions(
    interactions: Query<(&Interaction, &MenuAction), Changed<Interaction>>,
    time: Res<Time>,
    mut menu: ResMut<GameMenuState>,
    mut chess_match: ResMut<ChessMatch>,
    mut ai_task: ResMut<AiTask>,
    mut auto_turn: ResMut<CameraAutoTurn>,
    mut camera: Single<&mut PanOrbitCamera>,
) {
    for (interaction, action) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *action {
            MenuAction::Mode(mode) => menu.selected_mode = mode,
            MenuAction::Variant(variant) => {
                menu.selected_variant = variant;
                if chess_match.variant != variant {
                    restart_match(&mut chess_match, variant);
                    chess_match.controllers = [Controller::Human, Controller::Human];
                    ai_task.cancel();
                }
            }
            MenuAction::Side(side) => menu.selected_side = side,
            MenuAction::Multiplayer => {}
            MenuAction::Start => {
                let side = resolve_side(menu.selected_side, time.elapsed().as_nanos());
                let mode = menu.selected_mode;
                let variant = menu.selected_variant;
                restart_match(&mut chess_match, variant);
                chess_match.controllers = controllers_for(mode, side);
                ai_task.cancel();
                menu.active_mode = mode;
                menu.active_side = side;
                menu.open = false;
                start_camera_turn(&mut camera, &mut auto_turn, side);
            }
        }
    }
}

fn sync_menu_visibility(
    menu: Res<GameMenuState>,
    mut roots: Query<&mut Visibility, With<GameMenuRoot>>,
) {
    if !menu.is_changed() {
        return;
    }
    for mut visibility in &mut roots {
        *visibility = if menu.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn style_menu_buttons(
    menu: Res<GameMenuState>,
    mut buttons: Query<(
        &Interaction,
        &MenuAction,
        &mut BackgroundColor,
        &mut BorderColor,
        &Children,
    )>,
    mut labels: Query<&mut TextColor, With<MenuButtonLabel>>,
) {
    for (interaction, action, mut background, mut border, children) in &mut buttons {
        let unavailable = matches!(action, MenuAction::Multiplayer);
        let selected = action_selected(*action, &menu);
        let is_start = matches!(action, MenuAction::Start);
        let (background_color, border_color, label_color) = if unavailable {
            (
                Color::srgba(0.08, 0.07, 0.11, 0.45),
                Color::srgba(1.0, 1.0, 1.0, 0.05),
                Color::srgba(0.55, 0.52, 0.6, 0.55),
            )
        } else if is_start {
            (
                if *interaction == Interaction::Hovered {
                    ACCENT_HOVER
                } else {
                    ACCENT
                },
                Color::srgba(1.0, 0.7, 0.83, 0.88),
                Color::WHITE,
            )
        } else if selected {
            (
                BUTTON_SELECTED,
                Color::srgba(1.0, 0.32, 0.6, 0.95),
                TEXT_PRIMARY,
            )
        } else if *interaction == Interaction::Hovered {
            (
                BUTTON_HOVER,
                Color::srgba(1.0, 0.3, 0.58, 0.58),
                TEXT_PRIMARY,
            )
        } else {
            (BUTTON, Color::srgba(1.0, 1.0, 1.0, 0.1), TEXT_PRIMARY)
        };
        background.0 = background_color;
        *border = BorderColor::all(border_color);
        for child in children.iter() {
            if let Ok(mut text_color) = labels.get_mut(child) {
                text_color.0 = label_color;
            }
        }
    }
}

fn animate_menu_background(
    time: Res<Time>,
    menu: Res<GameMenuState>,
    mut camera: Single<&mut PanOrbitCamera>,
) {
    camera.enabled = !menu.open;
    if menu.open {
        camera.target_yaw += time.delta_secs() * MENU_ORBIT_SPEED;
        camera.force_update = true;
    }
}

fn action_selected(action: MenuAction, menu: &GameMenuState) -> bool {
    match action {
        MenuAction::Mode(mode) => menu.selected_mode == mode,
        MenuAction::Variant(variant) => menu.selected_variant == variant,
        MenuAction::Side(side) => menu.selected_side == side,
        MenuAction::Multiplayer | MenuAction::Start => false,
    }
}

fn resolve_side(choice: SideChoice, entropy: u128) -> Side {
    match choice {
        SideChoice::Random if entropy.is_multiple_of(2) => Side::White,
        SideChoice::Random => Side::Black,
        SideChoice::White => Side::White,
        SideChoice::Black => Side::Black,
    }
}

fn controllers_for(mode: GameMode, human_side: Side) -> [Controller; 2] {
    match mode {
        GameMode::Local => [Controller::Human, Controller::Human],
        GameMode::Ai => std::array::from_fn(|index| {
            if index == human_side.index() {
                Controller::Human
            } else {
                Controller::Computer
            }
        }),
    }
}

fn variant_label(variant: Variant) -> &'static str {
    match variant {
        Variant::Capablanca => "Capablanca",
        Variant::Gothic => "Gothic",
        Variant::Embassy => "Embassy",
        Variant::Schoolbook => "Schoolbook",
        Variant::Bird => "Bird",
        Variant::Carrera => "Carrera",
        Variant::Grand => "Grand Chess",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_defaults_match_product_defaults() {
        let menu = GameMenuState::default();
        assert_eq!(menu.selected_mode, GameMode::Local);
        assert_eq!(menu.selected_variant, Variant::Gothic);
        assert_eq!(menu.selected_side, SideChoice::Random);
    }

    #[test]
    fn ai_assigns_exactly_one_human_controller() {
        assert_eq!(
            controllers_for(GameMode::Ai, Side::White),
            [Controller::Human, Controller::Computer]
        );
        assert_eq!(
            controllers_for(GameMode::Ai, Side::Black),
            [Controller::Computer, Controller::Human]
        );
    }
}
