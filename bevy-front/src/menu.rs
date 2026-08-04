use bevy::{
    ecs::hierarchy::ChildSpawnerCommands,
    input_focus::tab_navigation::TabIndex,
    picking::hover::Hovered,
    prelude::*,
    text::{EditableText, EditableTextFilter, TextCursorStyle},
    ui_widgets::{
        Slider, SliderDragState, SliderPrecision, SliderRange, SliderStep, SliderThumb,
        SliderValue, TrackClick, observe, slider_self_update,
    },
    window::PrimaryWindow,
};
use bevy_panorbit_camera::PanOrbitCamera;
use capablanca_chess_plus::{Color as Side, Variant};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

use crate::{
    ai::{
        AiSettings, AiTask, DEFAULT_DIFFICULTY, MAX_DIFFICULTY, MIN_DIFFICULTY,
        difficulty_description,
    },
    app::FrontendSet::Menu,
    game::{ChessMatch, Controller, restart_match},
    multiplayer::{MultiplayerCommand, MultiplayerPhase, MultiplayerState},
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
                    sync_ai_controls_visibility,
                    sync_multiplayer_controls_visibility,
                    reset_multiplayer_input_on_menu_open,
                    sync_browser_multiplayer_input,
                    sync_multiplayer_game_id,
                    sync_multiplayer_status,
                    sync_start_button_label,
                    sync_ai_difficulty,
                    adapt_menu_layout,
                    style_menu_buttons,
                    style_ai_difficulty_slider,
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
    Multiplayer,
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
    pub(crate) multiplayer_game_id: String,
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
            multiplayer_game_id: String::new(),
        }
    }
}

#[derive(Component)]
struct GameMenuRoot;

#[derive(Component)]
struct GameMenuPanel;

#[derive(Component)]
struct MenuSubtitle;

#[derive(Component)]
struct MenuTextSize(f32);

#[derive(Component)]
struct ResponsiveMenuGap {
    row: f32,
    column: f32,
}

#[derive(Component, Clone, Copy)]
enum MenuAction {
    Mode(GameMode),
    Variant(Variant),
    Side(SideChoice),
    Start,
}

#[derive(Component)]
struct MenuButtonLabel;

#[derive(Component)]
struct AiDifficultyControls;

#[derive(Component)]
struct AiDifficultySlider;

#[derive(Component)]
struct AiDifficultySliderThumb;

#[derive(Component)]
struct AiDifficultySliderFill;

#[derive(Component)]
struct AiDifficultyValueLabel;

#[derive(Component)]
struct MultiplayerControls;

#[derive(Component)]
struct MultiplayerGameIdInput;

#[derive(Component)]
struct MultiplayerStatusLabel;

#[derive(Component)]
struct StartButtonLabel;

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
                GameMenuPanel,
            ))
            .with_children(|panel| {
                panel.spawn(text(&font, "CAPABLANCA CHESS +", 32.0, ACCENT));
                panel.spawn((
                    text(
                        &font,
                        "Configure a game while the board waits behind the glass.",
                        14.0,
                        TEXT_MUTED,
                    ),
                    MenuSubtitle,
                ));

                spawn_section_label(panel, &font, "GAME MODE");
                spawn_row(panel, |row| {
                    spawn_menu_button(
                        row,
                        &font,
                        "Local",
                        MenuAction::Mode(GameMode::Local),
                        percent(30),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "AI",
                        MenuAction::Mode(GameMode::Ai),
                        percent(30),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "Multiplayer",
                        MenuAction::Mode(GameMode::Multiplayer),
                        percent(30),
                    );
                });

                spawn_section_label(panel, &font, "RULES & STARTING POSITION");
                panel
                    .spawn((
                        Node {
                            width: percent(100),
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: px(8),
                            row_gap: px(8),
                            ..default()
                        },
                        ResponsiveMenuGap {
                            row: 8.0,
                            column: 8.0,
                        },
                    ))
                    .with_children(|grid| {
                        for variant in Variant::ALL {
                            spawn_menu_button(
                                grid,
                                &font,
                                variant_label(variant),
                                MenuAction::Variant(variant),
                                percent(23),
                            );
                        }
                    });

                spawn_ai_difficulty_controls(panel, &font);
                spawn_multiplayer_controls(panel, &font);

                spawn_section_label(panel, &font, "PLAY AS");
                spawn_row(panel, |row| {
                    spawn_menu_button(
                        row,
                        &font,
                        "Random",
                        MenuAction::Side(SideChoice::Random),
                        percent(30),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "White",
                        MenuAction::Side(SideChoice::White),
                        percent(30),
                    );
                    spawn_menu_button(
                        row,
                        &font,
                        "Black",
                        MenuAction::Side(SideChoice::Black),
                        percent(30),
                    );
                });

                spawn_menu_button(panel, &font, "START GAME", MenuAction::Start, percent(100));
            });
        });
}

fn spawn_multiplayer_controls(parent: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    parent
        .spawn((
            Node {
                display: Display::None,
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                ..default()
            },
            MultiplayerControls,
            ResponsiveMenuGap {
                row: 8.0,
                column: 0.0,
            },
        ))
        .with_children(|controls| {
            controls.spawn(text(
                font,
                "GAME ID · LEAVE EMPTY TO CREATE",
                12.0,
                Color::srgba(1.0, 0.48, 0.7, 0.9),
            ));
            controls.spawn((
                Node {
                    width: percent(100),
                    min_height: px(44),
                    padding: UiRect::axes(px(14), px(9)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(12)),
                    overflow: Overflow::clip_x(),
                    ..default()
                },
                EditableText {
                    max_characters: Some(12),
                    visible_width: Some(12.0),
                    allow_newlines: false,
                    ..default()
                },
                EditableTextFilter::new(|character| character.is_ascii_alphanumeric()),
                TextLayout::no_wrap(),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(TEXT_PRIMARY),
                TextCursorStyle {
                    color: ACCENT,
                    ..default()
                },
                BackgroundColor(BUTTON),
                BorderColor::all(Color::srgba(1.0, 0.35, 0.62, 0.42)),
                TabIndex(0),
                MultiplayerGameIdInput,
                MenuTextSize(18.0),
            ));
            controls.spawn((
                text(
                    font,
                    "Enter a game id to join, or leave it empty to create one.",
                    12.0,
                    TEXT_MUTED,
                ),
                MultiplayerStatusLabel,
            ));
        });
}

fn spawn_ai_difficulty_controls(parent: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    parent
        .spawn((
            Node {
                display: Display::None,
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(9),
                ..default()
            },
            AiDifficultyControls,
            ResponsiveMenuGap {
                row: 9.0,
                column: 0.0,
            },
        ))
        .with_children(|controls| {
            controls
                .spawn((
                    Node {
                        width: percent(100),
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        row_gap: px(3),
                        ..default()
                    },
                    ResponsiveMenuGap {
                        row: 3.0,
                        column: 6.0,
                    },
                ))
                .with_children(|header| {
                    header.spawn(text(
                        font,
                        "FAIRY-STOCKFISH STRENGTH",
                        12.0,
                        Color::srgba(1.0, 0.48, 0.7, 0.9),
                    ));
                    header.spawn((
                        text(
                            font,
                            &difficulty_description(DEFAULT_DIFFICULTY),
                            12.0,
                            TEXT_PRIMARY,
                        ),
                        AiDifficultyValueLabel,
                    ));
                });

            controls
                .spawn((
                    Node {
                        width: percent(100),
                        height: px(28),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    Hovered::default(),
                    Slider {
                        track_click: TrackClick::Snap,
                        ..default()
                    },
                    SliderValue(f32::from(DEFAULT_DIFFICULTY)),
                    SliderRange::new(f32::from(MIN_DIFFICULTY), f32::from(MAX_DIFFICULTY)),
                    SliderStep(1.0),
                    SliderPrecision(0),
                    AiDifficultySlider,
                    observe(slider_self_update),
                ))
                .with_children(|slider| {
                    slider.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(8),
                            right: px(8),
                            top: px(11.5),
                            height: px(5),
                            border_radius: BorderRadius::all(px(3)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.14)),
                        Pickable::IGNORE,
                    ));
                    slider
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(8),
                                right: px(8),
                                top: px(11.5),
                                height: px(5),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_child((
                            Node {
                                width: percent(0),
                                height: percent(100),
                                border_radius: BorderRadius::all(px(3)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(1.0, 0.22, 0.55, 0.84)),
                            Pickable::IGNORE,
                            AiDifficultySliderFill,
                        ));
                    slider
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(8),
                                right: px(8),
                                top: px(0),
                                bottom: px(0),
                                justify_content: JustifyContent::SpaceBetween,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_children(|ticks| {
                            for _ in MIN_DIFFICULTY..=MAX_DIFFICULTY {
                                ticks.spawn((
                                    Node {
                                        width: px(3),
                                        height: px(3),
                                        border_radius: BorderRadius::MAX,
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.48)),
                                ));
                            }
                        });
                    slider
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(0),
                                right: px(16),
                                top: px(0),
                                bottom: px(0),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_child((
                            SliderThumb,
                            AiDifficultySliderThumb,
                            Node {
                                position_type: PositionType::Absolute,
                                left: percent(0),
                                top: px(6),
                                width: px(16),
                                height: px(16),
                                border: UiRect::all(px(2)),
                                border_radius: BorderRadius::MAX,
                                ..default()
                            },
                            BackgroundColor(ACCENT),
                            BorderColor::all(Color::srgba(1.0, 0.72, 0.85, 0.95)),
                        ));
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
        .spawn((
            Node {
                width: percent(100),
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(8),
                row_gap: px(8),
                ..default()
            },
            ResponsiveMenuGap {
                row: 8.0,
                column: 8.0,
            },
        ))
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
    let mut button = parent.spawn((
        Button,
        Node {
            width,
            min_width: px(0),
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
    ));
    if is_start {
        button.with_child((
            text(font, label, 14.0, TEXT_PRIMARY),
            MenuButtonLabel,
            StartButtonLabel,
        ));
    } else {
        button.with_child((text(font, label, 14.0, TEXT_PRIMARY), MenuButtonLabel));
    }
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
        MenuTextSize(size),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MenuLayout {
    Regular,
    Compact,
    Tiny,
}

fn menu_layout_for_size(width: f32, height: f32) -> MenuLayout {
    if width < 380.0 || height < 440.0 {
        MenuLayout::Tiny
    } else if width < 640.0 || height < 680.0 {
        MenuLayout::Compact
    } else {
        MenuLayout::Regular
    }
}

#[allow(clippy::too_many_arguments)]
fn adapt_menu_layout(
    window: Single<&Window, With<PrimaryWindow>>,
    mut previous: Local<Option<MenuLayout>>,
    mut nodes: ParamSet<(
        Query<&mut Node, With<GameMenuRoot>>,
        Query<&mut Node, (With<GameMenuPanel>, Without<GameMenuRoot>)>,
        Query<(&MenuAction, &mut Node), (Without<GameMenuRoot>, Without<GameMenuPanel>)>,
        Query<(&ResponsiveMenuGap, &mut Node), (Without<GameMenuRoot>, Without<GameMenuPanel>)>,
        Query<&mut Node, With<MultiplayerGameIdInput>>,
        Query<&mut Node, With<MenuSubtitle>>,
    )>,
    mut text: Query<(&MenuTextSize, &mut TextFont)>,
) {
    let layout = menu_layout_for_size(window.width(), window.height());
    if *previous == Some(layout) {
        return;
    }
    *previous = Some(layout);

    let (
        root_padding,
        panel_padding_x,
        panel_padding_y,
        panel_gap,
        scale,
        button_height,
        start_height,
    ) = match layout {
        MenuLayout::Regular => (18.0, 34.0, 28.0, 17.0, 1.0, 44.0, 54.0),
        MenuLayout::Compact => (8.0, 14.0, 12.0, 9.0, 0.84, 38.0, 46.0),
        MenuLayout::Tiny => (4.0, 8.0, 4.0, 4.0, 0.7, 28.0, 34.0),
    };
    let gap_scale = match layout {
        MenuLayout::Regular => 1.0,
        MenuLayout::Compact => 0.7,
        MenuLayout::Tiny => 0.42,
    };

    for mut root in &mut nodes.p0() {
        root.padding = UiRect::all(px(root_padding));
    }
    for mut panel in &mut nodes.p1() {
        panel.width = percent(if layout == MenuLayout::Regular {
            92
        } else {
            98
        });
        panel.max_height = percent(if layout == MenuLayout::Regular {
            96
        } else {
            99
        });
        panel.padding = UiRect::axes(px(panel_padding_x), px(panel_padding_y));
        panel.row_gap = px(panel_gap);
    }
    for (action, mut button) in &mut nodes.p2() {
        let is_start = matches!(action, MenuAction::Start);
        button.min_width = px(0);
        button.height = px(if is_start {
            start_height
        } else {
            button_height
        });
        button.padding = UiRect::axes(
            px(12.0 * scale),
            px(if layout == MenuLayout::Tiny {
                2.0
            } else {
                8.0 * scale
            }),
        );
    }
    for (size, mut font) in &mut text {
        font.font_size = FontSize::Px(size.0 * scale);
    }
    for mut subtitle in &mut nodes.p5() {
        subtitle.display = if layout == MenuLayout::Regular {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (gap, mut node) in &mut nodes.p3() {
        node.row_gap = px(gap.row * gap_scale);
        node.column_gap = px(gap.column * gap_scale);
    }
    for mut input in &mut nodes.p4() {
        input.min_height = px(match layout {
            MenuLayout::Regular => 44.0,
            MenuLayout::Compact => 38.0,
            MenuLayout::Tiny => 30.0,
        });
        input.padding = UiRect::axes(px(14.0 * scale), px(9.0 * scale));
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_menu_interactions(
    interactions: Query<(&Interaction, &MenuAction), Changed<Interaction>>,
    time: Res<Time>,
    mut menu: ResMut<GameMenuState>,
    mut chess_match: ResMut<ChessMatch>,
    mut ai_task: NonSendMut<AiTask>,
    mut auto_turn: ResMut<CameraAutoTurn>,
    mut camera: Single<&mut PanOrbitCamera>,
    multiplayer: Res<MultiplayerState>,
    mut multiplayer_commands: MessageWriter<MultiplayerCommand>,
) {
    for (interaction, action) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *action {
            MenuAction::Mode(mode) => {
                menu.selected_mode = mode;
                if mode == GameMode::Ai {
                    ai_task.warm_up();
                }
            }
            MenuAction::Variant(variant) => {
                menu.selected_variant = variant;
                if chess_match.variant != variant {
                    restart_match(&mut chess_match, variant);
                    chess_match.controllers = [Controller::Human, Controller::Human];
                    ai_task.cancel();
                }
            }
            MenuAction::Side(side) => menu.selected_side = side,
            MenuAction::Start => {
                let mode = menu.selected_mode;
                if mode == GameMode::Multiplayer {
                    if matches!(
                        multiplayer.phase,
                        MultiplayerPhase::Connecting | MultiplayerPhase::Reconnecting
                    ) {
                        continue;
                    }
                    let game_id = menu.multiplayer_game_id.trim().to_ascii_uppercase();
                    if game_id.is_empty() {
                        multiplayer_commands.write(MultiplayerCommand::Create {
                            variant: menu.selected_variant,
                            side: menu.selected_side,
                        });
                    } else {
                        multiplayer_commands.write(MultiplayerCommand::Join { game_id });
                    }
                    continue;
                }
                let side =
                    starting_camera_side(mode, menu.selected_side, time.elapsed().as_nanos());
                let variant = menu.selected_variant;
                restart_match(&mut chess_match, variant);
                chess_match.controllers = controllers_for(mode, side);
                match mode {
                    GameMode::Ai => ai_task.start_new_game(),
                    GameMode::Local => ai_task.shut_down(),
                    GameMode::Multiplayer => unreachable!(),
                }
                multiplayer_commands.write(MultiplayerCommand::Disconnect);
                menu.active_mode = mode;
                menu.active_side = side;
                menu.open = false;
                start_camera_turn(&mut camera, &mut auto_turn, side);
            }
        }
    }
}

fn sync_multiplayer_controls_visibility(
    menu: Res<GameMenuState>,
    mut controls: Query<&mut Node, With<MultiplayerControls>>,
) {
    if !menu.is_changed() {
        return;
    }
    for mut node in &mut controls {
        node.display = if menu.selected_mode == GameMode::Multiplayer {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn sync_multiplayer_game_id(
    inputs: Query<&EditableText, (Changed<EditableText>, With<MultiplayerGameIdInput>)>,
    mut menu: ResMut<GameMenuState>,
) {
    for input in &inputs {
        let value = input.value().to_string();
        if menu.multiplayer_game_id != value {
            menu.multiplayer_game_id = value;
        }
    }
}

fn reset_multiplayer_input_on_menu_open(
    menu: Res<GameMenuState>,
    mut was_open: Local<bool>,
    mut inputs: Query<&mut EditableText, With<MultiplayerGameIdInput>>,
) {
    let just_opened = menu.open && !*was_open;
    *was_open = menu.open;
    if !just_opened {
        return;
    }
    for mut input in &mut inputs {
        if input.value().to_string() != menu.multiplayer_game_id {
            input.clear();
            input.editor_mut().set_text(&menu.multiplayer_game_id);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_browser_multiplayer_input() {}

#[cfg(target_arch = "wasm32")]
fn sync_browser_multiplayer_input(
    mut menu: ResMut<GameMenuState>,
    mut inputs: Query<
        (&mut EditableText, &ComputedNode, &UiGlobalTransform),
        With<MultiplayerGameIdInput>,
    >,
    mut was_visible: Local<bool>,
) {
    let Some(input_element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("capablanca-game-id-input"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return;
    };
    let visible = menu.open && menu.selected_mode == GameMode::Multiplayer;
    if !visible {
        if *was_visible {
            let _ = input_element.style().set_property("display", "none");
            let _ = input_element.blur();
        }
        *was_visible = false;
        return;
    }

    let Ok((mut editable, computed, transform)) = inputs.single_mut() else {
        return;
    };
    if computed.is_empty() {
        let _ = input_element.style().set_property("display", "none");
        return;
    }

    let inverse_scale = computed.inverse_scale_factor();
    let size = computed.size() * inverse_scale;
    let (_, _, center) = transform.to_scale_angle_translation();
    let center = center * inverse_scale;
    let top_left = center - size * 0.5;
    let style = input_element.style();
    let _ = style.set_property("display", "block");
    let _ = style.set_property("left", &format!("{}px", top_left.x));
    let _ = style.set_property("top", &format!("{}px", top_left.y));
    let _ = style.set_property("width", &format!("{}px", size.x));
    let _ = style.set_property("height", &format!("{}px", size.y));

    if !*was_visible {
        input_element.set_value(&menu.multiplayer_game_id);
    }
    let value = normalize_game_id_input(&input_element.value());
    if input_element.value() != value {
        input_element.set_value(&value);
    }
    if editable.value().to_string() != value {
        editable.clear();
        editable.editor_mut().set_text(&value);
    }
    if menu.multiplayer_game_id != value {
        menu.multiplayer_game_id = value;
    }
    *was_visible = true;
}

#[cfg(any(target_arch = "wasm32", test))]
fn normalize_game_id_input(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

fn sync_multiplayer_status(
    multiplayer: Res<MultiplayerState>,
    mut labels: Query<&mut Text, With<MultiplayerStatusLabel>>,
) {
    if !multiplayer.is_changed() {
        return;
    }
    for mut label in &mut labels {
        **label = multiplayer.status.clone();
    }
}

fn sync_start_button_label(
    menu: Res<GameMenuState>,
    mut labels: Query<&mut Text, With<StartButtonLabel>>,
) {
    if !menu.is_changed() {
        return;
    }
    let label = if menu.selected_mode != GameMode::Multiplayer {
        "START GAME"
    } else if menu.multiplayer_game_id.trim().is_empty() {
        "CREATE GAME"
    } else {
        "JOIN GAME"
    };
    for mut text in &mut labels {
        **text = label.to_owned();
    }
}

fn sync_ai_controls_visibility(
    menu: Res<GameMenuState>,
    mut controls: Query<&mut Node, With<AiDifficultyControls>>,
) {
    if !menu.is_changed() {
        return;
    }
    for mut node in &mut controls {
        node.display = if menu.selected_mode == GameMode::Ai {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn sync_ai_difficulty(
    sliders: Query<&SliderValue, (Changed<SliderValue>, With<AiDifficultySlider>)>,
    mut settings: ResMut<AiSettings>,
) {
    for value in &sliders {
        settings.set_difficulty(value.0.round() as u8);
    }
}

#[allow(clippy::type_complexity)]
fn style_ai_difficulty_slider(
    sliders: Query<
        (
            Entity,
            &SliderValue,
            &SliderRange,
            &Hovered,
            &SliderDragState,
        ),
        (
            With<AiDifficultySlider>,
            Or<(
                Changed<SliderValue>,
                Changed<Hovered>,
                Changed<SliderDragState>,
            )>,
        ),
    >,
    children: Query<&Children>,
    mut thumbs: Query<
        (&mut Node, &mut BackgroundColor),
        (
            With<AiDifficultySliderThumb>,
            Without<AiDifficultySliderFill>,
        ),
    >,
    mut fills: Query<
        &mut Node,
        (
            With<AiDifficultySliderFill>,
            Without<AiDifficultySliderThumb>,
        ),
    >,
    mut labels: Query<&mut Text, With<AiDifficultyValueLabel>>,
) {
    for (slider, value, range, hovered, drag_state) in &sliders {
        let position = range.thumb_position(value.0) * 100.0;
        for descendant in children.iter_descendants(slider) {
            if let Ok((mut node, mut background)) = thumbs.get_mut(descendant) {
                node.left = percent(position);
                background.0 = if hovered.0 || drag_state.dragging {
                    ACCENT_HOVER
                } else {
                    ACCENT
                };
            }
            if let Ok(mut node) = fills.get_mut(descendant) {
                node.width = percent(position);
            }
        }
        for mut label in &mut labels {
            **label = difficulty_description(value.0.round() as u8);
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
        let selected = action_selected(*action, &menu);
        let is_start = matches!(action, MenuAction::Start);
        let (background_color, border_color, label_color) = if is_start {
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
        MenuAction::Start => false,
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

fn starting_camera_side(mode: GameMode, choice: SideChoice, entropy: u128) -> Side {
    match mode {
        // Pass-and-play always starts from White, who makes the first move.
        // The camera will switch to the other side after that move finishes.
        GameMode::Local => Side::White,
        GameMode::Ai | GameMode::Multiplayer => resolve_side(choice, entropy),
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
        GameMode::Multiplayer => std::array::from_fn(|index| {
            if index == human_side.index() {
                Controller::Human
            } else {
                Controller::Remote
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

    #[test]
    fn local_always_starts_from_the_white_side() {
        for choice in [SideChoice::Random, SideChoice::White, SideChoice::Black] {
            assert_eq!(
                starting_camera_side(GameMode::Local, choice, 1),
                Side::White
            );
        }
    }

    #[test]
    fn ai_starting_side_respects_the_players_choice() {
        assert_eq!(
            starting_camera_side(GameMode::Ai, SideChoice::White, 1),
            Side::White
        );
        assert_eq!(
            starting_camera_side(GameMode::Ai, SideChoice::Black, 0),
            Side::Black
        );
    }

    #[test]
    fn browser_game_id_input_is_sanitized_and_uppercase() {
        assert_eq!(
            normalize_game_id_input(" ab-c23_deF456789 "),
            "ABC23DEF4567"
        );
    }
}
