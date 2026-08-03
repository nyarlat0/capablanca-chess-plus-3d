use bevy::{audio::Volume, prelude::*};
use capablanca_chess_plus::GameOutcome;

use crate::{
    app::FrontendSet,
    game::{ChessMatch, Controller},
    pieces::PieceAnimationState,
};

const MOVE_SOUND_PATHS: [&str; 4] = [
    "sounds/chess_move_-01.ogg",
    "sounds/chess_move_-02.ogg",
    "sounds/chess_move_-03.ogg",
    "sounds/chess_move_-04.ogg",
];

/// Linear volume of chess move sound effects. `1.0` is the source volume.
const MOVE_SOUND_VOLUME: f32 = 0.75;
/// Linear volume of victory and defeat sound effects.
const RESULT_SOUND_VOLUME: f32 = 0.75;
/// Lets the short move sound finish before a game-result sound begins.
const RESULT_SOUND_DELAY_SECONDS: f32 = 0.24;

pub(crate) struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MoveSoundAssets>()
            .init_resource::<MoveSoundState>()
            .add_systems(Update, update_game_audio.in_set(FrontendSet::Audio));
    }
}

#[derive(Resource)]
struct MoveSoundAssets {
    clips: Vec<Handle<AudioSource>>,
    victory: Handle<AudioSource>,
    defeat: Handle<AudioSource>,
}

impl FromWorld for MoveSoundAssets {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self {
            clips: MOVE_SOUND_PATHS
                .into_iter()
                .map(|path| asset_server.load(path))
                .collect(),
            victory: asset_server.load("sounds/victory.ogg"),
            defeat: asset_server.load("sounds/defeat.ogg"),
        }
    }
}

#[derive(Resource, Default)]
struct MoveSoundState {
    played_move_generation: Option<u64>,
    previous_clip: Option<usize>,
    pending_result: Option<PendingResultSound>,
}

struct PendingResultSound {
    generation: u64,
    timer: Timer,
    sound: ResultSound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResultSound {
    Victory,
    Defeat,
}

fn update_game_audio(
    mut commands: Commands,
    time: Res<Time>,
    chess_match: Res<ChessMatch>,
    animation: Res<PieceAnimationState>,
    sounds: Res<MoveSoundAssets>,
    mut state: ResMut<MoveSoundState>,
) {
    update_pending_result(&mut commands, &time, &chess_match, &sounds, &mut state);

    if state.played_move_generation == Some(chess_match.generation) {
        return;
    }

    // The board's primary moving piece marks the generation when it reaches
    // its destination. Captured-piece tray motion is intentionally independent.
    if chess_match.last_move.is_none()
        || sounds.clips.is_empty()
        || !animation.has_move_landed(chess_match.generation)
    {
        return;
    }

    state.played_move_generation = Some(chess_match.generation);
    let clip_index = random_clip_index(sounds.clips.len(), state.previous_clip);
    state.previous_clip = Some(clip_index);
    play_one_shot(
        &mut commands,
        sounds.clips[clip_index].clone(),
        MOVE_SOUND_VOLUME,
    );

    if let Some(sound) = result_sound(chess_match.game.outcome(), chess_match.controllers) {
        state.pending_result = Some(PendingResultSound {
            generation: chess_match.generation,
            timer: Timer::from_seconds(RESULT_SOUND_DELAY_SECONDS, TimerMode::Once),
            sound,
        });
    }
}

fn update_pending_result(
    commands: &mut Commands,
    time: &Time,
    chess_match: &ChessMatch,
    sounds: &MoveSoundAssets,
    state: &mut MoveSoundState,
) {
    let Some(pending) = &mut state.pending_result else {
        return;
    };
    if pending.generation != chess_match.generation {
        state.pending_result = None;
        return;
    }
    if !pending.timer.tick(time.delta()).just_finished() {
        return;
    }

    let sound = pending.sound;
    state.pending_result = None;
    let clip = match sound {
        ResultSound::Victory => sounds.victory.clone(),
        ResultSound::Defeat => sounds.defeat.clone(),
    };
    play_one_shot(commands, clip, RESULT_SOUND_VOLUME);
}

fn play_one_shot(commands: &mut Commands, clip: Handle<AudioSource>, volume: f32) {
    commands.spawn((
        AudioPlayer::new(clip),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
    ));
}

fn result_sound(outcome: GameOutcome, controllers: [Controller; 2]) -> Option<ResultSound> {
    match outcome {
        GameOutcome::Win { winner } => Some(if controllers[winner.index()] == Controller::Human {
            ResultSound::Victory
        } else {
            ResultSound::Defeat
        }),
        GameOutcome::Draw(_) => Some(ResultSound::Defeat),
        GameOutcome::Ongoing | GameOutcome::Check => None,
    }
}

fn random_clip_index(clip_count: usize, previous: Option<usize>) -> usize {
    match previous.filter(|_| clip_count > 1) {
        Some(previous) => {
            // Pick among every clip except the one just played, preventing an
            // audible repeat while keeping every remaining variant equiprobable.
            let offset = fastrand::usize(1..clip_count);
            (previous + offset) % clip_count
        }
        None => fastrand::usize(..clip_count),
    }
}

#[cfg(test)]
mod tests {
    use capablanca_chess_plus::{Color as Side, DrawReason};

    use super::*;

    #[test]
    fn ai_result_is_judged_from_the_human_side() {
        let controllers = [Controller::Computer, Controller::Human];
        assert_eq!(
            result_sound(
                GameOutcome::Win {
                    winner: Side::Black,
                },
                controllers,
            ),
            Some(ResultSound::Victory)
        );
        assert_eq!(
            result_sound(
                GameOutcome::Win {
                    winner: Side::White,
                },
                controllers,
            ),
            Some(ResultSound::Defeat)
        );
    }

    #[test]
    fn local_win_is_a_victory_and_every_draw_is_a_defeat() {
        let controllers = [Controller::Human, Controller::Human];
        assert_eq!(
            result_sound(
                GameOutcome::Win {
                    winner: Side::White,
                },
                controllers,
            ),
            Some(ResultSound::Victory)
        );
        assert_eq!(
            result_sound(GameOutcome::Draw(DrawReason::Stalemate), controllers,),
            Some(ResultSound::Defeat)
        );
    }
}
