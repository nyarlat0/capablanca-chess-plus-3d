mod backend;

use bevy::prelude::*;
use capablanca_chess_plus::Variant;

use self::backend::{BackendEvent, FairyBackend, TeraBackend};
use crate::{
    app::FrontendSet,
    game::{
        ChessMatch, Controller, MoveAnalysis, apply_move, is_playable, outcome_message, side_name,
    },
    menu::GameMenuState,
    pieces::PieceAnimationState,
};

pub(crate) const MIN_SEARCH_LEVEL: u8 = 1;
pub(crate) const MAX_SEARCH_LEVEL: u8 = 10;
pub(crate) const DEFAULT_SEARCH_LEVEL: u8 = 5;

// The UI deliberately reports this real per-move search budget instead of
// inventing an Elo mapping. Node limits also behave consistently across native
// and browser builds, unlike wall-clock limits on heavily throttled tabs.
const NODE_BUDGETS: [u64; 10] = [
    1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000,
];

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
    search_level: u8,
}

impl AiSettings {
    pub(crate) const fn search_level(&self) -> u8 {
        self.search_level
    }

    pub(crate) fn set_search_level(&mut self, level: u8) {
        self.search_level = level.clamp(MIN_SEARCH_LEVEL, MAX_SEARCH_LEVEL);
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            search_level: DEFAULT_SEARCH_LEVEL,
        }
    }
}

pub(crate) fn search_budget_description(level: u8) -> String {
    let profile = SearchProfile::new(level);
    format!(
        "Level {} · {} nodes / move",
        profile.level,
        format_nodes(profile.nodes)
    )
}

struct SearchProfile {
    level: u8,
    nodes: u64,
}

impl SearchProfile {
    fn new(level: u8) -> Self {
        let level = level.clamp(MIN_SEARCH_LEVEL, MAX_SEARCH_LEVEL);
        let index = usize::from(level - MIN_SEARCH_LEVEL);
        Self {
            level,
            nodes: NODE_BUDGETS[index],
        }
    }
}

fn format_nodes(nodes: u64) -> String {
    if nodes >= 1_000_000 && nodes.is_multiple_of(1_000_000) {
        format!("{}M", nodes / 1_000_000)
    } else if nodes >= 1_000 && nodes.is_multiple_of(1_000) {
        format!("{}k", nodes / 1_000)
    } else if nodes >= 1_000 {
        format!("{:.1}k", nodes as f64 / 1_000.0)
    } else {
        nodes.to_string()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EngineKind {
    Fairy,
    Tera,
}

impl EngineKind {
    const fn for_variant(variant: Variant) -> Self {
        if matches!(variant, Variant::TerachessII) {
            Self::Tera
        } else {
            Self::Fairy
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Fairy => "Fairy-Stockfish",
            Self::Tera => "TeraStockfish",
        }
    }
}

enum ActiveBackend {
    Fairy(FairyBackend),
    Tera(TeraBackend),
}

impl ActiveBackend {
    fn new(kind: EngineKind) -> Result<Self, String> {
        match kind {
            EngineKind::Fairy => FairyBackend::new().map(Self::Fairy),
            EngineKind::Tera => TeraBackend::new().map(Self::Tera),
        }
    }

    fn send(&self, command: &str) -> Result<(), String> {
        match self {
            Self::Fairy(backend) => backend.send(command),
            Self::Tera(backend) => backend.send(command),
        }
    }

    fn drain(&self, destination: &mut Vec<BackendEvent>) {
        match self {
            Self::Fairy(backend) => backend.drain(destination),
            Self::Tera(backend) => backend.drain(destination),
        }
    }
}

pub(crate) struct AiTask {
    backend: Option<ActiveBackend>,
    engine_kind: Option<EngineKind>,
    state: EngineState,
    new_game_pending: bool,
}

impl Default for AiTask {
    fn default() -> Self {
        Self {
            backend: None,
            engine_kind: None,
            state: EngineState::Dormant,
            new_game_pending: true,
        }
    }
}

impl AiTask {
    pub(crate) fn warm_up_for(&mut self, variant: Variant) {
        let kind = EngineKind::for_variant(variant);
        if self.engine_kind == Some(kind)
            && !matches!(self.state, EngineState::Dormant | EngineState::Failed(_))
        {
            return;
        }
        self.backend = None;
        self.engine_kind = Some(kind);
        self.state = EngineState::Booting;
        match ActiveBackend::new(kind) {
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
            if self.engine_kind == Some(EngineKind::Tera) {
                // A browser worker cannot receive `stop` while executing a
                // synchronous WebAssembly call. Termination is immediate and
                // prevents a stale long search from consuming resources.
                self.backend = None;
                self.engine_kind = None;
                self.state = EngineState::Dormant;
                self.new_game_pending = true;
                return;
            }
            if let Err(error) = self.send("stop") {
                self.fail(error);
            } else {
                self.state = EngineState::Stopping;
            }
        }
    }

    pub(crate) fn start_new_game(&mut self, variant: Variant) {
        self.cancel();
        self.new_game_pending = true;
        self.warm_up_for(variant);
    }

    pub(crate) fn shut_down(&mut self) {
        self.backend = None;
        self.engine_kind = None;
        self.state = EngineState::Dormant;
        self.new_game_pending = true;
    }

    fn send(&self, command: &str) -> Result<(), String> {
        self.backend
            .as_ref()
            .ok_or_else(|| "AI engine is unavailable".to_owned())?
            .send(command)
    }

    fn fail(&mut self, message: String) {
        let engine = self.engine_kind.map_or("AI engine", EngineKind::name);
        error!("{engine} integration failed: {message}");
        self.state = EngineState::Failed(message);
    }

    fn failure(&self) -> Option<&str> {
        match &self.state {
            EngineState::Failed(message) => Some(message),
            _ => None,
        }
    }

    fn engine_name(&self) -> &'static str {
        self.engine_kind.map_or("AI engine", EngineKind::name)
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
        let status = format!("{} is unavailable: {error}", task.engine_name());
        if chess_match.status != status {
            chess_match.status = status;
        }
        return;
    }
    let required_engine = EngineKind::for_variant(chess_match.variant);
    if task.engine_kind != Some(required_engine) || matches!(task.state, EngineState::Dormant) {
        task.warm_up_for(chess_match.variant);
        return;
    }
    if !matches!(task.state, EngineState::Idle) {
        return;
    }

    let profile = SearchProfile::new(settings.search_level());
    let mut commands = vec![format!(
        "setoption name UCI_Variant value {}",
        uci_variant(chess_match.variant)
    )];
    if task.new_game_pending {
        commands.push("ucinewgame".to_owned());
    }
    commands.extend([
        format!("position fen {}", chess_match.game.position().to_fen()),
        format!("go nodes {}", profile.nodes),
    ]);

    for command in commands {
        if let Err(error) = task.send(&command) {
            task.fail(error);
            return;
        }
    }
    task.new_game_pending = false;
    chess_match.status = format!(
        "{} {} is searching {} nodes…",
        side_name(side),
        task.engine_name(),
        format_nodes(profile.nodes)
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
            let engine = task.engine_name();
            task.fail(format!(
                "{engine} returned illegal move {best_move}: {error}"
            ));
            chess_match.status = format!("{engine} error: illegal move {best_move}.");
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

const fn uci_variant(variant: Variant) -> &'static str {
    match variant {
        Variant::Classic => "chess",
        Variant::Capablanca => "capablanca",
        Variant::Gothic => "gothic",
        Variant::Embassy => "embassy",
        Variant::Schoolbook => "ccp_schoolbook",
        Variant::Bird => "ccp_bird",
        Variant::Carrera => "ccp_carrera",
        Variant::Grand => "grand",
        Variant::Shako => "shako",
        Variant::Pemba => "ccp_pemba",
        Variant::TerachessII => "terachessii",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_budget_is_honest_and_clamped() {
        assert_eq!(SearchProfile::new(0).level, MIN_SEARCH_LEVEL);
        assert_eq!(SearchProfile::new(5).nodes, 25_000);
        assert_eq!(SearchProfile::new(99).level, MAX_SEARCH_LEVEL);
        assert_eq!(SearchProfile::new(MAX_SEARCH_LEVEL).nodes, 1_000_000);
        assert_eq!(search_budget_description(5), "Level 5 · 25k nodes / move");
    }

    #[test]
    fn all_frontend_variants_have_a_uci_name() {
        assert_eq!(
            Variant::ALL.map(uci_variant),
            [
                "capablanca",
                "gothic",
                "embassy",
                "ccp_schoolbook",
                "ccp_bird",
                "ccp_carrera",
                "grand",
                "shako",
                "ccp_pemba",
                "terachessii",
                "chess",
            ]
        );
        assert_eq!(
            EngineKind::for_variant(Variant::TerachessII),
            EngineKind::Tera
        );
        assert_eq!(EngineKind::for_variant(Variant::Gothic), EngineKind::Fairy);
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

        fn wait_for_line(backend: &FairyBackend, predicate: impl Fn(&str) -> bool) -> String {
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

        let backend = FairyBackend::new().expect("bundled Fairy-Stockfish starts");
        backend.send("uci").unwrap();
        wait_for_line(&backend, |line| line == "uciok");
        backend.send("setoption name Use NNUE value false").unwrap();

        for variant in Variant::ALL
            .into_iter()
            .filter(|variant| *variant != Variant::TerachessII)
        {
            backend
                .send(&format!(
                    "setoption name UCI_Variant value {}",
                    uci_variant(variant)
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
