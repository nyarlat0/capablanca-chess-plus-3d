use bevy::prelude::*;
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
                    handle_open_menu,
                    open_hud_on_room_created,
                    open_hud_on_game_end,
                    sync_hud_visibility,
                    update_hud,
                    style_open_menu_button,
                    style_hud_toggle,
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
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            expanded: false,
            last_outcome: GameOutcome::Ongoing,
        }
    }
}

#[derive(Component)]
struct HudRoot;

#[derive(Component)]
struct HudPanel;

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct OpenMenuButton;

#[derive(Component)]
struct HudToggleButton;

#[derive(Component)]
struct HudToggleIcon;

fn setup_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font: Handle<Font> = asset_server.load("fonts/FiraSans-Bold.ttf");
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(8),
                right: px(8),
                align_items: AlignItems::FlexStart,
                column_gap: px(8),
                ..default()
            },
            GlobalZIndex(20),
            Visibility::Hidden,
            HudRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: px(330),
                    padding: UiRect::all(px(16)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(18)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(12),
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

            root.spawn((
                Button,
                Node {
                    width: px(38),
                    height: px(38),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(11)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.12, 0.12, 0.15, 0.3)),
                BorderColor::all(TOGGLE_GRAY),
                HudToggleButton,
            ))
            .with_child((
                Text::new("‹"),
                TextFont {
                    font: font.into(),
                    font_size: FontSize::Px(25.0),
                    ..default()
                },
                TextColor(TOGGLE_GRAY),
                Pickable::IGNORE,
                HudToggleIcon,
            ));
        });
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
    mut hud: Query<&mut Text, With<HudText>>,
) {
    if menu.open {
        return;
    }
    let Ok(mut text) = hud.single_mut() else {
        return;
    };
    **text = hud_text(
        chess_match.game.position().rules().name(),
        menu.active_mode,
        chess_match.game.outcome(),
        multiplayer.game_id.as_deref(),
    );
}

fn hud_text(
    game_type: &str,
    mode: GameMode,
    outcome: GameOutcome,
    game_id: Option<&str>,
) -> String {
    let mode = match mode {
        GameMode::Local => "Local · two players",
        GameMode::Ai => "AI · versus computer",
        GameMode::Multiplayer => "Multiplayer · online",
    };
    let mut value = format!("{game_type}\n{mode}");
    if let Some(game_id) = game_id {
        value.push_str(&format!("\nGame ID · {game_id}"));
    }
    if !is_playable(outcome) {
        value.push_str(&format!("\n\n{}", outcome_message(outcome)));
    }
    value
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
        let (background_color, foreground_color) = match interaction {
            Interaction::None => (Color::srgba(0.12, 0.12, 0.15, 0.3), TOGGLE_GRAY),
            Interaction::Hovered => (Color::srgba(0.2, 0.2, 0.23, 0.5), TOGGLE_GRAY_HOVER),
            Interaction::Pressed => (Color::srgba(0.08, 0.08, 0.1, 0.65), TOGGLE_GRAY_HOVER),
        };
        background.0 = background_color;
        *border = BorderColor::all(foreground_color);
        for child in children.iter() {
            if let Ok(mut icon) = icons.get_mut(child) {
                icon.0 = foreground_color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use capablanca_chess_plus::{Color as Side, DrawReason, Game, Position, Variant};

    use super::*;

    #[test]
    fn active_game_hud_contains_only_its_mode() {
        assert_eq!(
            hud_text("Gothic Chess", GameMode::Local, GameOutcome::Ongoing, None,),
            "Gothic Chess\nLocal · two players"
        );
        assert_eq!(
            hud_text("Capablanca Chess", GameMode::Ai, GameOutcome::Check, None,),
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
                None,
            ),
            "Gothic Chess\nAI · versus computer\n\nCheckmate — White wins."
        );
        assert_eq!(
            hud_text(
                "Capablanca Chess",
                GameMode::Local,
                GameOutcome::Draw(DrawReason::Stalemate),
                None,
            ),
            "Capablanca Chess\nLocal · two players\n\nDraw by stalemate."
        );
    }

    #[test]
    fn multiplayer_hud_always_contains_the_public_game_id() {
        assert_eq!(
            hud_text(
                "Gothic Chess",
                GameMode::Multiplayer,
                GameOutcome::Ongoing,
                Some("ABC234DEFG"),
            ),
            "Gothic Chess\nMultiplayer · online\nGame ID · ABC234DEFG"
        );
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
