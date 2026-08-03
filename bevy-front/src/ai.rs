mod backend;

use bevy::prelude::*;
use capablanca_chess_plus::Variant;

use self::backend::{Backend, BackendEvent};
use crate::{
    app::FrontendSet,
    game::{
        ChessMatch, Controller, MoveAnalysis, apply_move, is_playable, outcome_message, side_name,
    },
    menu::GameMenuState,
    pieces::PieceAnimationState,
};

pub(crate) const MIN_DIFFICULTY: u8 = 1;
pub(crate) const MAX_DIFFICULTY: u8 = 10;
pub(crate) const DEFAULT_DIFFICULTY: u8 = 5;

// Approximate UCI Elo targets. Fairy-Stockfish's calibration is based on
// orthodox chess, so the displayed values are deliberately marked as estimates.
const DIFFICULTY_ELO: [u16; 9] = [500, 750, 1000, 1250, 1500, 1750, 2000, 2250, 2550];
const MOVE_TIME_MS: [u32; 10] = [150, 200, 300, 450, 650, 900, 1_250, 1_700, 2_300, 3_200];

pub(crate) struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiSettings>()
            .init_non_send::<AiTask>()
            .add_systems(Update, poll_ai_engine.in_set(FrontendSet::AiPoll))
            .add_systems(Update, start_ai_search.in_set(FrontendSet::AiStart));
    }
}

#[derive(Resource)]
pub(crate) struct AiSettings {
    difficulty: u8,
}

impl AiSettings {
    pub(crate) const fn difficulty(&self) -> u8 {
        self.difficulty
    }

    pub(crate) fn set_difficulty(&mut self, difficulty: u8) {
        self.difficulty = difficulty.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY);
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            difficulty: DEFAULT_DIFFICULTY,
        }
    }
}

pub(crate) fn difficulty_description(difficulty: u8) -> String {
    let profile = DifficultyProfile::new(difficulty);
    profile.elo.map_or_else(
        || format!("Level {} · Maximum", profile.level),
        |elo| format!("Level {} · ~{elo} Elo", profile.level),
    )
}

struct DifficultyProfile {
    level: u8,
    elo: Option<u16>,
    move_time_ms: u32,
}

impl DifficultyProfile {
    fn new(level: u8) -> Self {
        let level = level.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY);
        let index = usize::from(level - MIN_DIFFICULTY);
        Self {
            level,
            elo: DIFFICULTY_ELO.get(index).copied(),
            move_time_ms: MOVE_TIME_MS[index],
        }
    }
}

pub(crate) struct AiTask {
    backend: Option<Backend>,
    state: EngineState,
    new_game_pending: bool,
}

impl Default for AiTask {
    fn default() -> Self {
        Self {
            backend: None,
            state: EngineState::Dormant,
            new_game_pending: true,
        }
    }
}

impl AiTask {
    pub(crate) fn warm_up(&mut self) {
        if !matches!(self.state, EngineState::Dormant | EngineState::Failed(_)) {
            return;
        }
        self.backend = None;
        self.state = EngineState::Booting;
        match Backend::new() {
            Ok(backend) => {
                self.backend = Some(backend);
                if let Err(error) = self.send("uci") {
                    self.fail(error);
                }
            }
            Err(error) => self.fail(error),
        }
    }

    pub(crate) fn cancel(&mut self) {
        if matches!(self.state, EngineState::Searching(_)) {
            if let Err(error) = self.send("stop") {
                self.fail(error);
            } else {
                self.state = EngineState::Stopping;
            }
        }
    }

    pub(crate) fn start_new_game(&mut self) {
        self.cancel();
        self.new_game_pending = true;
        self.warm_up();
    }

    pub(crate) fn shut_down(&mut self) {
        self.backend = None;
        self.state = EngineState::Dormant;
        self.new_game_pending = true;
    }

    fn send(&self, command: &str) -> Result<(), String> {
        self.backend
            .as_ref()
            .ok_or_else(|| "Fairy-Stockfish is unavailable".to_owned())?
            .send(command)
    }

    fn fail(&mut self, message: String) {
        error!("Fairy-Stockfish integration failed: {message}");
        self.state = EngineState::Failed(message);
    }

    fn failure(&self) -> Option<&str> {
        match &self.state {
            EngineState::Failed(message) => Some(message),
            _ => None,
        }
    }
}

enum EngineState {
    Dormant,
    Booting,
    WaitingUntilReady,
    Idle,
    Searching(SearchInFlight),
    Stopping,
    Failed(String),
}

struct SearchInFlight {
    generation: u64,
    analysis: MoveAnalysis,
}

fn start_ai_search(
    mut chess_match: ResMut<ChessMatch>,
    menu: Res<GameMenuState>,
    animation: Res<PieceAnimationState>,
    settings: Res<AiSettings>,
    mut task: NonSendMut<AiTask>,
) {
    if menu.open
        || !animation.is_settled(chess_match.generation)
        || chess_match.pending_promotion.is_some()
        || !is_playable(chess_match.game.outcome())
    {
        return;
    }
    let side = chess_match.game.position().side_to_move();
    if chess_match.controllers[side.index()] != Controller::Computer {
        return;
    }

    if let Some(error) = task.failure() {
        let status = format!("Fairy-Stockfish is unavailable: {error}");
        if chess_match.status != status {
            chess_match.status = status;
        }
        return;
    }
    if !matches!(task.state, EngineState::Idle) {
        return;
    }

    let profile = DifficultyProfile::new(settings.difficulty());
    let limit_strength = profile.elo.is_some();
    let mut commands = vec![
        format!(
            "setoption name UCI_Variant value {}",
            fairy_variant(chess_match.variant)
        ),
        format!(
            "setoption name UCI_LimitStrength value {}",
            if limit_strength { "true" } else { "false" }
        ),
        "setoption name Skill Level value 20".to_owned(),
    ];
    if let Some(elo) = profile.elo {
        commands.push(format!("setoption name UCI_Elo value {elo}"));
    }
    if task.new_game_pending {
        commands.push("ucinewgame".to_owned());
    }
    commands.extend([
        format!("position fen {}", chess_match.game.position().to_fen()),
        format!("go movetime {}", profile.move_time_ms),
    ]);

    for command in commands {
        if let Err(error) = task.send(&command) {
            task.fail(error);
            return;
        }
    }
    task.new_game_pending = false;
    chess_match.status = format!(
        "{} Fairy-Stockfish is thinking at level {}…",
        side_name(side),
        profile.level
    );
    task.state = EngineState::Searching(SearchInFlight {
        generation: chess_match.generation,
        analysis: MoveAnalysis::default(),
    });
}

fn poll_ai_engine(
    menu: Res<GameMenuState>,
    mut chess_match: ResMut<ChessMatch>,
    mut task: NonSendMut<AiTask>,
) {
    let mut events = Vec::new();
    if let Some(backend) = &task.backend {
        backend.drain(&mut events);
    }
    for event in events {
        match event {
            BackendEvent::Line(line) => {
                handle_engine_line(&line, &menu, &mut chess_match, &mut task);
            }
            BackendEvent::Error(message) => task.fail(message),
        }
    }
}

fn handle_engine_line(
    line: &str,
    menu: &GameMenuState,
    chess_match: &mut ChessMatch,
    task: &mut AiTask,
) {
    if line == "uciok" && matches!(task.state, EngineState::Booting) {
        for command in [
            "setoption name Threads value 1",
            "setoption name Hash value 32",
            "isready",
        ] {
            if let Err(error) = task.send(command) {
                task.fail(error);
                return;
            }
        }
        task.state = EngineState::WaitingUntilReady;
        return;
    }

    if line == "readyok" && matches!(task.state, EngineState::WaitingUntilReady) {
        task.state = EngineState::Idle;
        return;
    }

    if line.starts_with("info ") {
        if let EngineState::Searching(search) = &mut task.state {
            update_analysis(line, &mut search.analysis);
        }
        return;
    }

    let Some(best_move) = line
        .strip_prefix("bestmove ")
        .and_then(|value| value.split_whitespace().next())
    else {
        return;
    };

    if matches!(task.state, EngineState::Stopping) {
        if let Err(error) = task.send("isready") {
            task.fail(error);
        } else {
            task.state = EngineState::WaitingUntilReady;
        }
        return;
    }

    let EngineState::Searching(search) = &task.state else {
        return;
    };
    let generation = search.generation;
    let analysis = search.analysis;
    task.state = EngineState::Idle;

    if menu.open || generation != chess_match.generation {
        return;
    }
    if best_move == "(none)" || best_move == "0000" {
        chess_match.status = outcome_message(chess_match.game.outcome());
        return;
    }
    match chess_match.game.position().parse_uci_move(best_move) {
        Ok(chess_move) => apply_move(chess_match, chess_move, Some(&analysis)),
        Err(error) => {
            task.fail(format!(
                "Fairy-Stockfish returned illegal move {best_move}: {error}"
            ));
            chess_match.status = format!("Fairy-Stockfish error: illegal move {best_move}.");
        }
    }
}

fn update_analysis(line: &str, analysis: &mut MoveAnalysis) {
    let fields: Vec<_> = line.split_whitespace().collect();
    let mut index = 0;
    while index < fields.len() {
        match fields[index] {
            "depth" if index + 1 < fields.len() => {
                if let Ok(depth) = fields[index + 1].parse() {
                    analysis.depth = depth;
                }
                index += 2;
            }
            "nodes" if index + 1 < fields.len() => {
                if let Ok(nodes) = fields[index + 1].parse() {
                    analysis.nodes = nodes;
                }
                index += 2;
            }
            "score" if index + 2 < fields.len() => {
                if let Ok(value) = fields[index + 2].parse::<i32>() {
                    analysis.score = match fields[index + 1] {
                        "cp" => value,
                        "mate" => value.signum() * (1_000_000 - value.unsigned_abs() as i32),
                        _ => analysis.score,
                    };
                }
                index += 3;
            }
            _ => index += 1,
        }
    }
}

const fn fairy_variant(variant: Variant) -> &'static str {
    match variant {
        Variant::Capablanca => "capablanca",
        Variant::Gothic => "gothic",
        Variant::Embassy => "embassy",
        Variant::Schoolbook => "ccp_schoolbook",
        Variant::Bird => "ccp_bird",
        Variant::Carrera => "ccp_carrera",
        Variant::Grand => "grand",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_is_clamped_and_maximum_is_unlimited() {
        assert_eq!(DifficultyProfile::new(0).level, MIN_DIFFICULTY);
        assert_eq!(DifficultyProfile::new(5).elo, Some(1500));
        assert_eq!(DifficultyProfile::new(99).level, MAX_DIFFICULTY);
        assert_eq!(DifficultyProfile::new(MAX_DIFFICULTY).elo, None);
    }

    #[test]
    fn all_frontend_variants_have_a_fairy_stockfish_name() {
        assert_eq!(
            Variant::ALL.map(fairy_variant),
            [
                "capablanca",
                "gothic",
                "embassy",
                "ccp_schoolbook",
                "ccp_bird",
                "ccp_carrera",
                "grand",
            ]
        );
    }

    #[test]
    fn uci_info_updates_available_analysis_fields() {
        let mut analysis = MoveAnalysis::default();
        update_analysis(
            "info depth 12 seldepth 18 score cp -43 nodes 123456 nps 500000",
            &mut analysis,
        );
        assert_eq!(analysis.score, -43);
        assert_eq!(analysis.depth, 12);
        assert_eq!(analysis.nodes, 123_456);
    }

    #[cfg(all(
        not(target_arch = "wasm32"),
        target_arch = "x86_64",
        target_os = "linux"
    ))]
    #[test]
    fn bundled_fairy_stockfish_returns_a_legal_move_for_every_variant() {
        use std::{thread, time::Duration, time::Instant};

        fn wait_for_line(backend: &Backend, predicate: impl Fn(&str) -> bool) -> String {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut events = Vec::new();
                backend.drain(&mut events);
                for event in events {
                    match event {
                        BackendEvent::Line(line) if predicate(&line) => return line,
                        BackendEvent::Line(_) => {}
                        BackendEvent::Error(error) => panic!("Fairy-Stockfish failed: {error}"),
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for Fairy-Stockfish"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }

        let backend = Backend::new().expect("bundled Fairy-Stockfish starts");
        backend.send("uci").unwrap();
        wait_for_line(&backend, |line| line == "uciok");
        backend.send("setoption name Use NNUE value false").unwrap();

        for variant in Variant::ALL {
            backend
                .send(&format!(
                    "setoption name UCI_Variant value {}",
                    fairy_variant(variant)
                ))
                .unwrap();
            backend.send("isready").unwrap();
            wait_for_line(&backend, |line| line == "readyok");

            let position = variant.starting_position();
            backend
                .send(&format!("position fen {}", position.to_fen()))
                .unwrap();
            backend.send("go depth 1").unwrap();
            let reply = wait_for_line(&backend, |line| line.starts_with("bestmove "));
            let best_move = reply
                .split_whitespace()
                .nth(1)
                .expect("bestmove contains a move");
            position.parse_uci_move(best_move).unwrap_or_else(|error| {
                panic!("Fairy-Stockfish returned illegal {variant:?} move {best_move}: {error}")
            });
        }
    }
}
