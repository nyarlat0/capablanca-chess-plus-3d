use bevy::prelude::*;
use capablanca_chess_plus::Color as Side;

use crate::{
    ai::AiSettings,
    app::FrontendSet,
    game::{ChessMatch, side_name},
};

pub(crate) struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_hud)
            .add_systems(Update, update_hud.in_set(FrontendSet::Hud));
    }
}

#[derive(Component)]
struct HudText;

fn setup_hud(mut commands: Commands, asset_server: Res<AssetServer>) {
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
