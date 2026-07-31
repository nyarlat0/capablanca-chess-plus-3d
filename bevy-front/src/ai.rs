use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures_lite::future},
};
use capablanca_chess_plus::{Engine, SearchLimits, SearchResult};

use crate::{
    app::FrontendSet,
    game::{ChessMatch, Controller, apply_move, is_playable, outcome_message, side_name},
    menu::GameMenuState,
};

const DEFAULT_SEARCH_DEPTH: u8 = 3;

pub(crate) struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiSettings>()
            .init_resource::<AiTask>()
            .add_systems(Update, poll_ai_task.in_set(FrontendSet::AiPoll))
            .add_systems(Update, start_ai_task.in_set(FrontendSet::AiStart));
    }
}

#[derive(Resource)]
pub(crate) struct AiSettings {
    pub(crate) depth: u8,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            depth: DEFAULT_SEARCH_DEPTH,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct AiTask(Option<Task<AiReply>>);

impl AiTask {
    pub(crate) fn cancel(&mut self) {
        self.0 = None;
    }
}

struct AiReply {
    generation: u64,
    result: Option<SearchResult>,
}

fn start_ai_task(
    mut chess_match: ResMut<ChessMatch>,
    menu: Res<GameMenuState>,
    settings: Res<AiSettings>,
    mut task: ResMut<AiTask>,
) {
    if menu.open
        || task.0.is_some()
        || chess_match.pending_promotion.is_some()
        || !is_playable(chess_match.game.outcome())
    {
        return;
    }
    let side = chess_match.game.position().side_to_move();
    if chess_match.controllers[side.index()] != Controller::Computer {
        return;
    }

    let position = chess_match.game.position().clone();
    let generation = chess_match.generation;
    let depth = settings.depth;
    chess_match.status = format!("{} computer is thinking at depth {depth}…", side_name(side));
    task.0 = Some(AsyncComputeTaskPool::get().spawn(async move {
        let result = Engine::new().search(&position, SearchLimits::depth(depth));
        AiReply { generation, result }
    }));
}

fn poll_ai_task(
    menu: Res<GameMenuState>,
    mut chess_match: ResMut<ChessMatch>,
    mut task: ResMut<AiTask>,
) {
    if menu.open {
        return;
    }
    let Some(ai_task) = task.0.as_mut() else {
        return;
    };
    let Some(reply) = future::block_on(future::poll_once(ai_task)) else {
        return;
    };
    task.0 = None;

    if reply.generation != chess_match.generation {
        return;
    }
    if let Some(result) = reply.result {
        apply_move(&mut chess_match, result.best_move, Some(&result));
    } else {
        chess_match.status = outcome_message(chess_match.game.outcome());
    }
}
