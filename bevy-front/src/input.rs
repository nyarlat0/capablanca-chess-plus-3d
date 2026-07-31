use bevy::prelude::*;

use crate::{
    app::FrontendSet,
    game::{ChessMatch, side_to_move_message},
    menu::GameMenuState,
};

pub(crate) struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_keyboard.in_set(FrontendSet::Input));
    }
}

fn handle_keyboard(
    keyboard: Res<ButtonInput<KeyCode>>,
    menu: Res<GameMenuState>,
    mut chess_match: ResMut<ChessMatch>,
) {
    if menu.open {
        return;
    }
    if chess_match.pending_promotion.is_some() {
        return;
    }

    if keyboard.just_pressed(KeyCode::Escape) {
        chess_match.selected = None;
        chess_match.status = side_to_move_message(&chess_match);
    }
}
