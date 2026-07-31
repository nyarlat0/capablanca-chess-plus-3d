use bevy::prelude::*;
use capablanca_chess_plus::{Color as Side, PieceKind, Variant};

use crate::{
    ai::{AiSettings, AiTask},
    app::FrontendSet,
    game::{
        ChessMatch, apply_move, promotion_prompt, restart_match, side_name, side_to_move_message,
    },
};

pub(crate) struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_keyboard.in_set(FrontendSet::Input));
    }
}

fn handle_keyboard(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut chess_match: ResMut<ChessMatch>,
    mut ai_settings: ResMut<AiSettings>,
    mut ai_task: ResMut<AiTask>,
) {
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

    if keyboard.just_pressed(KeyCode::KeyN) || keyboard.just_pressed(KeyCode::Numpad0) {
        let variant = chess_match.variant;
        restart_match(&mut chess_match, variant);
        ai_task.cancel();
    }

    if let Some(variant) = variant_key(&keyboard)
        && variant != chess_match.variant
    {
        restart_match(&mut chess_match, variant);
        ai_task.cancel();
    }

    let toggle_white =
        keyboard.just_pressed(KeyCode::Digit1) || keyboard.just_pressed(KeyCode::Numpad1);
    let toggle_black =
        keyboard.just_pressed(KeyCode::Digit2) || keyboard.just_pressed(KeyCode::Numpad2);
    if toggle_white || toggle_black {
        let index = usize::from(toggle_black);
        chess_match.controllers[index].toggle();
        chess_match.selected = None;
        chess_match.pending_promotion = None;
        chess_match.status = format!(
            "{} is now controlled by the {}.",
            side_name(if index == 0 { Side::White } else { Side::Black }),
            chess_match.controllers[index].label().to_ascii_lowercase()
        );
        ai_task.cancel();
    }

    let increase_depth = keyboard.just_pressed(KeyCode::Equal)
        || keyboard.just_pressed(KeyCode::NumpadAdd)
        || keyboard.just_pressed(KeyCode::ArrowUp);
    let decrease_depth = keyboard.just_pressed(KeyCode::Minus)
        || keyboard.just_pressed(KeyCode::NumpadSubtract)
        || keyboard.just_pressed(KeyCode::ArrowDown);
    let old_depth = ai_settings.depth;
    if increase_depth {
        ai_settings.increase_depth();
    }
    if decrease_depth {
        ai_settings.decrease_depth();
    }
    if ai_settings.depth != old_depth {
        chess_match.status = format!("Engine search depth set to {}.", ai_settings.depth);
        ai_task.cancel();
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

fn variant_key(keyboard: &ButtonInput<KeyCode>) -> Option<Variant> {
    [
        (KeyCode::F1, Variant::Capablanca),
        (KeyCode::F2, Variant::Gothic),
        (KeyCode::F3, Variant::Embassy),
        (KeyCode::F4, Variant::Schoolbook),
        (KeyCode::F5, Variant::Bird),
        (KeyCode::F6, Variant::Carrera),
        (KeyCode::F7, Variant::Grand),
    ]
    .into_iter()
    .find_map(|(key, variant)| keyboard.just_pressed(key).then_some(variant))
}
