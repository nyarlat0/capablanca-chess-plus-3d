use bevy::prelude::*;
use capablanca_chess_plus::PieceKind;

use crate::{
    app::FrontendSet,
    game::{ChessMatch, apply_move, promotion_prompt, side_to_move_message},
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
        if keyboard.just_pressed(KeyCode::Escape) {
            chess_match.pending_promotion = None;
            chess_match.selected = None;
            chess_match.status = "Promotion cancelled.".to_owned();
            return;
        }

        let requested = promotion_key(&keyboard);
        if requested.is_some() || keyboard.just_pressed(KeyCode::Space) {
            let promotion = requested.flatten();
            let chess_move = chess_match.pending_promotion.as_ref().and_then(|pending| {
                pending
                    .moves
                    .iter()
                    .copied()
                    .find(|candidate| candidate.promotion == promotion)
            });
            if let Some(chess_move) = chess_move {
                apply_move(&mut chess_match, chess_move, None);
            } else {
                chess_match.status = promotion_prompt(
                    &chess_match
                        .pending_promotion
                        .as_ref()
                        .expect("promotion is pending")
                        .moves,
                );
            }
            return;
        }
    }

    if keyboard.just_pressed(KeyCode::Escape) {
        chess_match.selected = None;
        chess_match.status = side_to_move_message(&chess_match);
    }
}

fn promotion_key(keyboard: &ButtonInput<KeyCode>) -> Option<Option<PieceKind>> {
    [
        (KeyCode::KeyQ, PieceKind::Queen),
        (KeyCode::KeyC, PieceKind::Chancellor),
        (KeyCode::KeyA, PieceKind::Archbishop),
        (KeyCode::KeyR, PieceKind::Rook),
        (KeyCode::KeyB, PieceKind::Bishop),
        (KeyCode::KeyN, PieceKind::Knight),
    ]
    .into_iter()
    .find_map(|(key, kind)| keyboard.just_pressed(key).then_some(Some(kind)))
}
