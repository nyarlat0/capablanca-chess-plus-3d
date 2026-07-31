use bevy::prelude::*;

use crate::{
    ai::{AiSettings, AiTask},
    app::FrontendSet,
    game::{ChessMatch, Controller, side_name},
    menu::{GameMenuState, GameMode},
};

const ACCENT: Color = Color::srgb(0.98, 0.19, 0.52);
const ACCENT_HOVER: Color = Color::srgb(1.0, 0.3, 0.61);
const PANEL: Color = Color::srgba(0.035, 0.025, 0.055, 0.84);

pub(crate) struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_hud).add_systems(
            Update,
            (
                handle_open_menu,
                sync_hud_visibility,
                update_hud,
                style_menu_button,
            )
                .chain()
                .in_set(FrontendSet::Hud),
        );
    }
}

#[derive(Component)]
struct HudRoot;

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct OpenMenuButton;

fn setup_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font: Handle<Font> = asset_server.load("fonts/FiraSans-Bold.ttf");
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(16),
                right: px(16),
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
            HudRoot,
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
                    BackgroundColor(Color::srgba(0.72, 0.08, 0.3, 0.72)),
                    BorderColor::all(Color::srgba(1.0, 0.38, 0.65, 0.78)),
                    OpenMenuButton,
                ))
                .with_child((
                    Text::new("NEW GAME / MENU"),
                    TextFont {
                        font: font.into(),
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Pickable::IGNORE,
                ));
        });
}

fn handle_open_menu(
    buttons: Query<&Interaction, (Changed<Interaction>, With<OpenMenuButton>)>,
    mut menu: ResMut<GameMenuState>,
    mut chess_match: ResMut<ChessMatch>,
    mut ai_task: ResMut<AiTask>,
) {
    if !buttons
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }

    menu.open = true;
    menu.selected_mode = menu.active_mode;
    menu.selected_variant = chess_match.variant;
    chess_match.controllers = [Controller::Human, Controller::Human];
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    ai_task.cancel();
}

fn sync_hud_visibility(menu: Res<GameMenuState>, mut roots: Query<&mut Visibility, With<HudRoot>>) {
    if !menu.is_changed() {
        return;
    }
    for mut visibility in &mut roots {
        *visibility = if menu.open {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}

fn update_hud(
    chess_match: Res<ChessMatch>,
    menu: Res<GameMenuState>,
    settings: Res<AiSettings>,
    mut hud: Query<&mut Text, With<HudText>>,
) {
    if menu.open {
        return;
    }
    let Ok(mut text) = hud.single_mut() else {
        return;
    };
    let position = chess_match.game.position();
    let matchup = match menu.active_mode {
        GameMode::Local => "Local · two players".to_owned(),
        GameMode::Ai => format!(
            "AI · playing {} · depth {}",
            side_name(menu.active_side),
            settings.depth
        ),
    };
    let selected = chess_match
        .selected
        .map_or_else(|| "—".to_owned(), |square| square.to_string());

    **text = format!(
        "{}\n{}\n{} to move · selected {}\n\n{}",
        position.rules().name(),
        matchup,
        side_name(position.side_to_move()),
        selected,
        chess_match.status,
    );
}

fn style_menu_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<OpenMenuButton>,
    >,
) {
    for (interaction, mut background, mut border) in &mut buttons {
        background.0 = match interaction {
            Interaction::Hovered => ACCENT_HOVER,
            Interaction::Pressed => ACCENT,
            Interaction::None => Color::srgba(0.72, 0.08, 0.3, 0.72),
        };
        *border = BorderColor::all(match interaction {
            Interaction::None => Color::srgba(1.0, 0.38, 0.65, 0.78),
            Interaction::Hovered | Interaction::Pressed => Color::srgba(1.0, 0.72, 0.84, 0.95),
        });
    }
}
