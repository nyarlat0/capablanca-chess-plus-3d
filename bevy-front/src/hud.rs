use bevy::clipboard::Clipboard;
use bevy::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use capablanca_chess_plus::GameOutcome;

use crate::{
    ai::AiTask,
    app::FrontendSet,
    game::{ChessMatch, Controller, is_playable, outcome_message},
    menu::{GameMenuState, GameMode},
    multiplayer::{MultiplayerCommand, MultiplayerRoomReady, MultiplayerState},
};

const ACCENT: Color = Color::srgb(0.98, 0.19, 0.52);
const ACCENT_HOVER: Color = Color::srgb(1.0, 0.3, 0.61);
const PANEL: Color = Color::srgba(0.035, 0.025, 0.055, 0.84);
const TOGGLE_GRAY: Color = Color::srgba(0.7, 0.7, 0.74, 0.58);
const TOGGLE_GRAY_HOVER: Color = Color::srgba(0.88, 0.88, 0.92, 0.9);

pub(crate) struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudState>()
            .add_systems(Startup, setup_hud)
            .add_systems(
                Update,
                (
                    handle_hud_toggle,
                    handle_copy_game_id,
                    handle_fullscreen_button,
                    handle_open_menu,
                    open_hud_on_room_created,
                    open_hud_on_game_end,
                    sync_hud_visibility,
                    update_hud,
                    update_copy_feedback,
                    sync_fullscreen_icon,
                    style_open_menu_button,
                    style_hud_toggle,
                    style_fullscreen_button,
                    style_copy_game_id_button,
                )
                    .chain()
                    .in_set(FrontendSet::Hud),
            );
    }
}

#[derive(Resource)]
struct HudState {
    expanded: bool,
    last_outcome: GameOutcome,
    copy_feedback: Option<(CopyFeedback, Timer)>,
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            expanded: false,
            last_outcome: GameOutcome::Ongoing,
            copy_feedback: None,
        }
    }
}

#[derive(Clone, Copy)]
enum CopyFeedback {
    Copied,
    Failed,
}

#[derive(Component)]
struct HudRoot;

#[derive(Component)]
struct HudPanel;

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct MultiplayerGameIdRow;

#[derive(Component)]
struct MultiplayerGameIdText;

#[derive(Component)]
struct CopyGameIdButton;

#[derive(Component)]
struct CopyGameIdButtonLabel;

#[derive(Component)]
struct OpenMenuButton;

#[derive(Component)]
struct HudToggleButton;

#[derive(Component)]
struct HudToggleIcon;

#[derive(Component)]
struct FullscreenButton;

#[derive(Component)]
struct FullscreenIconCorner {
    top: bool,
    left: bool,
}

fn setup_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font: Handle<Font> = asset_server.load("fonts/FiraSans-Bold.ttf");
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                right: px(0),
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            GlobalZIndex(20),
            Visibility::Hidden,
            HudRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: px(8),
                    right: px(54),
                    width: vw(72),
                    max_width: px(330),
                    max_height: vh(94),
                    padding: UiRect::all(px(16)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(18)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(12),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(Color::srgba(1.0, 0.38, 0.65, 0.26)),
                BoxShadow::new(
                    Color::srgba(0.0, 0.0, 0.0, 0.56),
                    px(0),
                    px(9),
                    px(3),
                    px(24),
                ),
                Visibility::Hidden,
                HudPanel,
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(""),
                    TextFont {
                        font: font.clone().into(),
                        font_size: FontSize::Px(15.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.94, 0.92, 0.97)),
                    Pickable::IGNORE,
                    HudText,
                ));
                panel
                    .spawn((
                        Node {
                            display: Display::None,
                            width: percent(100),
                            flex_wrap: FlexWrap::Wrap,
                            align_items: AlignItems::Center,
                            column_gap: px(8),
                            row_gap: px(7),
                            ..default()
                        },
                        MultiplayerGameIdRow,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Text::new(""),
                            TextFont {
                                font: font.clone().into(),
                                font_size: FontSize::Px(13.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.94, 0.92, 0.97)),
                            Node {
                                min_width: px(0),
                                flex_grow: 1.0,
                                ..default()
                            },
                            Pickable::IGNORE,
                            MultiplayerGameIdText,
                        ));
                        row.spawn((
                            Button,
                            Node {
                                min_width: px(76),
                                height: px(34),
                                flex_shrink: 0.0,
                                padding: UiRect::axes(px(10), px(5)),
                                border: UiRect::all(px(1)),
                                border_radius: BorderRadius::all(px(10)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.12, 0.09, 0.16, 0.76)),
                            BorderColor::all(Color::srgba(1.0, 0.38, 0.65, 0.72)),
                            CopyGameIdButton,
                            Name::new("Copy multiplayer game id"),
                        ))
                        .with_child((
                            Text::new("COPY ID"),
                            TextFont {
                                font: font.clone().into(),
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                            TextColor(Color::srgb(1.0, 0.74, 0.86)),
                            Pickable::IGNORE,
                            CopyGameIdButtonLabel,
                        ));
                    });
                panel
                    .spawn((
                        Button,
                        Node {
                            width: percent(100),
                            height: px(42),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(12)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(ACCENT),
                        BorderColor::all(Color::srgba(1.0, 0.68, 0.82, 0.82)),
                        OpenMenuButton,
                    ))
                    .with_child((
                        Text::new("NEW GAME"),
                        TextFont {
                            font: font.clone().into(),
                            font_size: FontSize::Px(13.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Pickable::IGNORE,
                    ));
            });

            root.spawn(Node {
                position_type: PositionType::Absolute,
                top: px(8),
                right: px(8),
                flex_direction: FlexDirection::Column,
                row_gap: px(7),
                ..default()
            })
            .with_children(|controls| {
                controls
                    .spawn((
                        Button,
                        corner_button_node(),
                        BackgroundColor(Color::srgba(0.12, 0.12, 0.15, 0.3)),
                        BorderColor::all(TOGGLE_GRAY),
                        HudToggleButton,
                        Name::new("Toggle game information"),
                    ))
                    .with_child((
                        Text::new("‹"),
                        TextFont {
                            font: font.clone().into(),
                            font_size: FontSize::Px(25.0),
                            ..default()
                        },
                        TextColor(TOGGLE_GRAY),
                        Pickable::IGNORE,
                        HudToggleIcon,
                    ));

                controls
                    .spawn((
                        Button,
                        corner_button_node(),
                        BackgroundColor(Color::srgba(0.12, 0.12, 0.15, 0.3)),
                        BorderColor::all(TOGGLE_GRAY),
                        FullscreenButton,
                        Name::new("Toggle fullscreen"),
                    ))
                    .with_children(|button| {
                        button
                            .spawn((
                                Node {
                                    width: px(17),
                                    height: px(17),
                                    position_type: PositionType::Relative,
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ))
                            .with_children(|icon| {
                                for (top, left) in
                                    [(true, true), (true, false), (false, true), (false, false)]
                                {
                                    icon.spawn((
                                        fullscreen_corner_node(top, left, false),
                                        BorderColor::all(TOGGLE_GRAY),
                                        Pickable::IGNORE,
                                        FullscreenIconCorner { top, left },
                                    ));
                                }
                            });
                    });
            });
        });
}

fn corner_button_node() -> Node {
    Node {
        width: px(38),
        height: px(38),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(11)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

fn fullscreen_corner_node(top: bool, left: bool, fullscreen: bool) -> Node {
    Node {
        position_type: PositionType::Absolute,
        top: if top { px(0) } else { Val::Auto },
        bottom: if top { Val::Auto } else { px(0) },
        left: if left { px(0) } else { Val::Auto },
        right: if left { Val::Auto } else { px(0) },
        width: px(7),
        height: px(7),
        border: fullscreen_corner_border(top, left, fullscreen),
        ..default()
    }
}

fn fullscreen_corner_border(top: bool, left: bool, fullscreen: bool) -> UiRect {
    let vertical_edge_on_left = left != fullscreen;
    let horizontal_edge_on_top = top != fullscreen;
    UiRect {
        left: if vertical_edge_on_left { px(2) } else { px(0) },
        right: if vertical_edge_on_left { px(0) } else { px(2) },
        top: if horizontal_edge_on_top { px(2) } else { px(0) },
        bottom: if horizontal_edge_on_top { px(0) } else { px(2) },
    }
}

fn handle_hud_toggle(
    buttons: Query<&Interaction, (Changed<Interaction>, With<HudToggleButton>)>,
    mut hud: ResMut<HudState>,
) {
    if buttons
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        hud.expanded = !hud.expanded;
    }
}

fn handle_copy_game_id(
    buttons: Query<&Interaction, (Changed<Interaction>, With<CopyGameIdButton>)>,
    multiplayer: Res<MultiplayerState>,
    mut clipboard: ResMut<Clipboard>,
    mut hud: ResMut<HudState>,
) {
    if !buttons
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }
    let Some(game_id) = multiplayer.game_id.as_deref() else {
        return;
    };

    let result = if let Err(error) = clipboard.set_text(game_id) {
        warn!("Could not copy multiplayer game id: {error}");
        CopyFeedback::Failed
    } else {
        CopyFeedback::Copied
    };
    hud.copy_feedback = Some((result, Timer::from_seconds(1.6, TimerMode::Once)));
}

#[cfg(not(target_arch = "wasm32"))]
fn handle_fullscreen_button(
    buttons: Query<&Interaction, (Changed<Interaction>, With<FullscreenButton>)>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    if !buttons
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }

    window.mode = toggled_window_mode(&window.mode);
}

#[cfg(not(target_arch = "wasm32"))]
fn toggled_window_mode(current: &WindowMode) -> WindowMode {
    if matches!(current, WindowMode::Windowed) {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_fullscreen_icon(
    window: Single<&Window, With<PrimaryWindow>>,
    mut previous: Local<Option<bool>>,
    mut corners: Query<(&FullscreenIconCorner, &mut Node)>,
) {
    let fullscreen = !matches!(window.mode, WindowMode::Windowed);
    update_fullscreen_icon(fullscreen, &mut previous, &mut corners);
}

#[cfg(target_arch = "wasm32")]
fn handle_fullscreen_button(
    buttons: Query<&Interaction, (Changed<Interaction>, With<FullscreenButton>)>,
) {
    if !buttons
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        error!("Browser Fullscreen API is unavailable: document not found");
        return;
    };
    if document.fullscreen_element().is_some() {
        document.exit_fullscreen();
        return;
    }
    let Some(root) = document.document_element() else {
        error!("Browser Fullscreen API is unavailable: document root not found");
        return;
    };
    if let Err(error) = root.request_fullscreen() {
        error!("Could not enter browser fullscreen: {error:?}");
    }
}

#[cfg(target_arch = "wasm32")]
fn sync_fullscreen_icon(
    mut previous: Local<Option<bool>>,
    mut corners: Query<(&FullscreenIconCorner, &mut Node)>,
) {
    let fullscreen = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.fullscreen_element())
        .is_some();
    update_fullscreen_icon(fullscreen, &mut previous, &mut corners);
}

fn update_fullscreen_icon(
    fullscreen: bool,
    previous: &mut Option<bool>,
    corners: &mut Query<(&FullscreenIconCorner, &mut Node)>,
) {
    if *previous == Some(fullscreen) {
        return;
    }
    *previous = Some(fullscreen);

    for (corner, mut node) in corners {
        node.border = fullscreen_corner_border(corner.top, corner.left, fullscreen);
    }
}

fn handle_open_menu(
    buttons: Query<&Interaction, (Changed<Interaction>, With<OpenMenuButton>)>,
    mut menu: ResMut<GameMenuState>,
    mut hud: ResMut<HudState>,
    mut chess_match: ResMut<ChessMatch>,
    mut ai_task: NonSendMut<AiTask>,
    mut multiplayer_commands: MessageWriter<MultiplayerCommand>,
) {
    if !buttons
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }

    menu.open = true;
    hud.expanded = false;
    menu.selected_mode = menu.active_mode;
    menu.selected_variant = chess_match.variant;
    chess_match.controllers = [Controller::Human, Controller::Human];
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    ai_task.cancel();
    multiplayer_commands.write(MultiplayerCommand::Disconnect);
    menu.multiplayer_game_id.clear();
}

fn open_hud_on_room_created(
    mut rooms: MessageReader<MultiplayerRoomReady>,
    mut hud: ResMut<HudState>,
) {
    if rooms.read().any(|room| room.created) {
        hud.expanded = true;
    }
}

fn open_hud_on_game_end(chess_match: Res<ChessMatch>, mut hud: ResMut<HudState>) {
    let outcome = chess_match.game.outcome();
    if outcome == hud.last_outcome {
        return;
    }

    let game_just_ended = is_playable(hud.last_outcome) && !is_playable(outcome);
    hud.last_outcome = outcome;
    if game_just_ended && !hud.expanded {
        hud.expanded = true;
    }
}

fn sync_hud_visibility(
    menu: Res<GameMenuState>,
    hud: Res<HudState>,
    mut roots: Query<&mut Visibility, (With<HudRoot>, Without<HudPanel>)>,
    mut panels: Query<&mut Visibility, (With<HudPanel>, Without<HudRoot>)>,
    mut icons: Query<&mut Text, With<HudToggleIcon>>,
) {
    if !menu.is_changed() && !hud.is_changed() {
        return;
    }
    for mut visibility in &mut roots {
        *visibility = if menu.open {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    for mut visibility in &mut panels {
        *visibility = if hud.expanded {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut icon in &mut icons {
        **icon = if hud.expanded { "›" } else { "‹" }.to_owned();
    }
}

fn update_hud(
    chess_match: Res<ChessMatch>,
    menu: Res<GameMenuState>,
    multiplayer: Res<MultiplayerState>,
    mut texts: ParamSet<(
        Query<&mut Text, With<HudText>>,
        Query<&mut Text, With<MultiplayerGameIdText>>,
    )>,
    mut game_id_rows: Query<&mut Node, With<MultiplayerGameIdRow>>,
) {
    if menu.open {
        return;
    }
    {
        let mut hud = texts.p0();
        let Ok(mut text) = hud.single_mut() else {
            return;
        };
        **text = hud_text(
            chess_match.game.position().rules().name(),
            menu.active_mode,
            chess_match.game.outcome(),
        );
    }
    let game_id = (menu.active_mode == GameMode::Multiplayer)
        .then(|| multiplayer.game_id.as_deref())
        .flatten();
    for mut row in &mut game_id_rows {
        row.display = if game_id.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut label in &mut texts.p1() {
        **label = game_id
            .map(|game_id| format!("GAME ID · {game_id}"))
            .unwrap_or_default();
    }
}

fn hud_text(game_type: &str, mode: GameMode, outcome: GameOutcome) -> String {
    let mode = match mode {
        GameMode::Local => "Local · two players",
        GameMode::Ai => "AI · versus computer",
        GameMode::Multiplayer => "Multiplayer · online",
    };
    let mut value = format!("{game_type}\n{mode}");
    if !is_playable(outcome) {
        value.push_str(&format!("\n\n{}", outcome_message(outcome)));
    }
    value
}

fn update_copy_feedback(
    time: Res<Time>,
    mut hud: ResMut<HudState>,
    mut labels: Query<&mut Text, With<CopyGameIdButtonLabel>>,
) {
    if let Some((_, timer)) = hud.copy_feedback.as_mut() {
        timer.tick(time.delta());
        if timer.is_finished() {
            hud.copy_feedback = None;
        }
    }
    let value = match hud.copy_feedback.as_ref().map(|(result, _)| result) {
        Some(CopyFeedback::Copied) => "COPIED",
        Some(CopyFeedback::Failed) => "FAILED",
        None => "COPY ID",
    };
    for mut label in &mut labels {
        if label.as_str() != value {
            **label = value.to_owned();
        }
    }
}

fn style_open_menu_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<OpenMenuButton>,
    >,
) {
    for (interaction, mut background, mut border) in &mut buttons {
        background.0 = match interaction {
            Interaction::None => ACCENT,
            Interaction::Hovered => ACCENT_HOVER,
            Interaction::Pressed => Color::srgb(0.84, 0.1, 0.4),
        };
        *border = BorderColor::all(match interaction {
            Interaction::None => Color::srgba(1.0, 0.68, 0.82, 0.82),
            Interaction::Hovered | Interaction::Pressed => Color::srgb(1.0, 0.8, 0.9),
        });
    }
}

fn style_hud_toggle(
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &Children,
        ),
        With<HudToggleButton>,
    >,
    mut icons: Query<&mut TextColor, With<HudToggleIcon>>,
) {
    for (interaction, mut background, mut border, children) in &mut buttons {
        let (background_color, foreground_color) = corner_button_colors(*interaction);
        background.0 = background_color;
        *border = BorderColor::all(foreground_color);
        for child in children.iter() {
            if let Ok(mut icon) = icons.get_mut(child) {
                icon.0 = foreground_color;
            }
        }
    }
}

fn style_fullscreen_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (With<FullscreenButton>, Without<FullscreenIconCorner>),
    >,
    mut corners: Query<&mut BorderColor, (With<FullscreenIconCorner>, Without<FullscreenButton>)>,
) {
    for (interaction, mut background, mut border) in &mut buttons {
        let (background_color, foreground_color) = corner_button_colors(*interaction);
        background.0 = background_color;
        *border = BorderColor::all(foreground_color);
        for mut corner in &mut corners {
            *corner = BorderColor::all(foreground_color);
        }
    }
}

fn style_copy_game_id_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<CopyGameIdButton>,
    >,
) {
    for (interaction, mut background, mut border) in &mut buttons {
        let (background_color, border_color) = match interaction {
            Interaction::None => (
                Color::srgba(0.12, 0.09, 0.16, 0.76),
                Color::srgba(1.0, 0.38, 0.65, 0.72),
            ),
            Interaction::Hovered => (Color::srgba(0.31, 0.11, 0.22, 0.9), ACCENT_HOVER),
            Interaction::Pressed => (Color::srgba(0.5, 0.07, 0.24, 0.96), Color::WHITE),
        };
        background.0 = background_color;
        *border = BorderColor::all(border_color);
    }
}

fn corner_button_colors(interaction: Interaction) -> (Color, Color) {
    match interaction {
        Interaction::None => (Color::srgba(0.12, 0.12, 0.15, 0.3), TOGGLE_GRAY),
        Interaction::Hovered => (Color::srgba(0.2, 0.2, 0.23, 0.5), TOGGLE_GRAY_HOVER),
        Interaction::Pressed => (Color::srgba(0.08, 0.08, 0.1, 0.65), TOGGLE_GRAY_HOVER),
    }
}

#[cfg(test)]
mod tests {
    use capablanca_chess_plus::{Color as Side, DrawReason, Game, Position, Variant};

    use super::*;

    #[test]
    fn active_game_hud_contains_only_its_mode() {
        assert_eq!(
            hud_text("Gothic Chess", GameMode::Local, GameOutcome::Ongoing),
            "Gothic Chess\nLocal · two players"
        );
        assert_eq!(
            hud_text("Capablanca Chess", GameMode::Ai, GameOutcome::Check),
            "Capablanca Chess\nAI · versus computer"
        );
    }

    #[test]
    fn finished_game_hud_adds_the_result() {
        assert_eq!(
            hud_text(
                "Gothic Chess",
                GameMode::Ai,
                GameOutcome::Win {
                    winner: Side::White
                },
            ),
            "Gothic Chess\nAI · versus computer\n\nCheckmate — White wins."
        );
        assert_eq!(
            hud_text(
                "Capablanca Chess",
                GameMode::Local,
                GameOutcome::Draw(DrawReason::Stalemate),
            ),
            "Capablanca Chess\nLocal · two players\n\nDraw by stalemate."
        );
    }

    #[test]
    fn multiplayer_hud_keeps_the_public_game_id_in_its_separate_copy_row() {
        assert_eq!(
            hud_text("Gothic Chess", GameMode::Multiplayer, GameOutcome::Ongoing,),
            "Gothic Chess\nMultiplayer · online"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fullscreen_button_toggles_borderless_mode() {
        let fullscreen = toggled_window_mode(&WindowMode::Windowed);
        assert!(matches!(
            fullscreen,
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        ));
        assert!(matches!(
            toggled_window_mode(&fullscreen),
            WindowMode::Windowed
        ));
    }

    #[test]
    fn fullscreen_icon_corners_turn_inward_when_active() {
        let expanded = fullscreen_corner_border(true, true, false);
        assert_eq!(expanded.left, px(2));
        assert_eq!(expanded.top, px(2));
        assert_eq!(expanded.right, px(0));
        assert_eq!(expanded.bottom, px(0));

        let collapsed = fullscreen_corner_border(true, true, true);
        assert_eq!(collapsed.left, px(0));
        assert_eq!(collapsed.top, px(0));
        assert_eq!(collapsed.right, px(2));
        assert_eq!(collapsed.bottom, px(2));
    }

    #[test]
    fn finishing_a_game_opens_the_hud_once() {
        let mut app = App::new();
        app.init_resource::<ChessMatch>()
            .init_resource::<HudState>()
            .add_systems(Update, open_hud_on_game_end);
        app.update();
        assert!(!app.world().resource::<HudState>().expanded);

        let checkmate = Position::from_fen(
            Variant::Capablanca.rules(),
            "k9/1Q8/2K7/10/10/10/10/10 b - - 0 1",
        )
        .expect("test checkmate position is valid");
        app.world_mut().resource_mut::<ChessMatch>().game = Game::new(checkmate);
        app.update();
        assert!(app.world().resource::<HudState>().expanded);

        app.world_mut().resource_mut::<HudState>().expanded = false;
        app.update();
        assert!(!app.world().resource::<HudState>().expanded);
    }
}
