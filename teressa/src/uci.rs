use crate::{HybridAgent, HybridOptions, VulkanAdvisor};
use capablanca_chess_plus::{Position, Variant};
use std::io::{self, BufRead};
use std::path::PathBuf;
use terastockfish::SearchHistory;

struct State {
    position: Position,
    history: SearchHistory,
    moves: Vec<capablanca_chess_plus::Move>,
    model_path: PathBuf,
    safety_nodes: u64,
    hash_megabytes: usize,
    threads: usize,
    agent: Option<HybridAgent<VulkanAdvisor>>,
}

impl Default for State {
    fn default() -> Self {
        let position = Variant::TerachessII.starting_position();
        Self {
            history: SearchHistory::new(&position),
            moves: Vec::new(),
            position,
            model_path: PathBuf::from("target/teressa/teressa-bdh"),
            safety_nodes: 50_000,
            hash_megabytes: 32,
            threads: 1,
            agent: None,
        }
    }
}

impl State {
    fn ensure_agent(&mut self) -> Result<&mut HybridAgent<VulkanAdvisor>, String> {
        if self.agent.is_none() {
            let advisor = VulkanAdvisor::load(&self.model_path)?;
            self.agent = Some(HybridAgent::new(
                advisor,
                HybridOptions {
                    safety_nodes: self.safety_nodes,
                    hash_megabytes: self.hash_megabytes,
                    search_threads: self.threads,
                    ..HybridOptions::default()
                },
            ));
        }
        Ok(self.agent.as_mut().expect("agent was initialized"))
    }

    fn reset_game(&mut self) {
        self.position = Variant::TerachessII.starting_position();
        self.history = SearchHistory::new(&self.position);
        self.moves.clear();
        if let Some(agent) = &mut self.agent {
            agent.reset();
        }
    }
}

/// Native UCI-compatible entrypoint. Teressa-specific explanations are emitted
/// as `info string` lines and therefore remain harmless to ordinary GUIs.
pub fn run() -> Result<(), String> {
    let mut state = State::default();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|error| format!("cannot read UCI command: {error}"))?;
        let command = line.trim();
        if command == "uci" {
            println!("id name Teressa 0.1");
            println!("id author Capablanca Chess Plus 3D contributors");
            println!("option name ModelPath type string default target/teressa/teressa-bdh");
            println!("option name SafetyNodes type spin default 50000 min 1 max 1000000000");
            println!("option name Hash type spin default 32 min 1 max 65536");
            println!("option name Threads type spin default 1 min 1 max 256");
            println!("uciok");
        } else if command == "isready" {
            println!("readyok");
        } else if command == "ucinewgame" {
            state.reset_game();
        } else if let Some(value) = command.strip_prefix("setoption ") {
            if let Err(error) = set_option(&mut state, value) {
                println!("info string invalid setoption command: {error}");
            }
        } else if let Some(value) = command.strip_prefix("position ") {
            if let Err(error) = set_position(&mut state, value) {
                println!("info string invalid position command: {error}");
            }
        } else if command == "go" || command.starts_with("go ") {
            let nodes = go_nodes(command).unwrap_or(state.safety_nodes).max(1);
            let position = state.position.clone();
            let history = state.history.clone();
            let recent_moves = state.moves.clone();
            let agent = match state.ensure_agent() {
                Ok(agent) => agent,
                Err(error) => {
                    println!("info string Teressa could not load its checkpoint: {error}");
                    println!("bestmove 0000");
                    continue;
                }
            };
            agent.set_safety_nodes(nodes);
            match agent.decide_with_recent_moves(&position, Some(&history), &recent_moves) {
                Ok(decision) => {
                    println!(
                        "info score cp {} string plan_ru {}",
                        decision.safety_score,
                        one_line(&decision.explanation_ru)
                    );
                    println!("info string plan_en {}", one_line(&decision.explanation_en));
                    if decision.safety_vetoed_policy_best {
                        println!(
                            "info string tactical safety veto selected {} instead of the raw neural policy",
                            decision.chess_move
                        );
                    }
                    println!("bestmove {}", decision.chess_move);
                }
                Err(error) => {
                    println!("info string Teressa failed to choose a move: {error}");
                    println!("bestmove 0000");
                }
            }
        } else if command == "stop" || command.is_empty() {
            // Searches are deliberately bounded and synchronous in v1.
        } else if command == "quit" {
            break;
        } else {
            println!("info string unsupported command: {command}");
        }
    }
    Ok(())
}

fn set_option(state: &mut State, value: &str) -> Result<(), String> {
    let Some(rest) = value.strip_prefix("name ") else {
        return Err("setoption requires `name`".to_owned());
    };
    let (name, value) = rest
        .split_once(" value ")
        .ok_or_else(|| "setoption requires `value`".to_owned())?;
    match name.to_ascii_lowercase().as_str() {
        "modelpath" => {
            state.model_path = PathBuf::from(value);
            state.agent = None;
        }
        "safetynodes" => {
            state.safety_nodes = parse(value, "SafetyNodes")?;
            if let Some(agent) = &mut state.agent {
                agent.set_safety_nodes(state.safety_nodes);
            }
        }
        "hash" => {
            state.hash_megabytes = parse(value, "Hash")?;
            if let Some(agent) = &mut state.agent {
                agent.resize_hash(state.hash_megabytes);
            }
        }
        "threads" => {
            state.threads = parse(value, "Threads")?;
            if let Some(agent) = &mut state.agent {
                agent.set_search_threads(state.threads);
            }
        }
        _ => return Err(format!("unknown UCI option `{name}`")),
    }
    Ok(())
}

fn set_position(state: &mut State, value: &str) -> Result<(), String> {
    let (position_spec, moves_spec) = value
        .split_once(" moves ")
        .map_or((value, None), |(position, moves)| (position, Some(moves)));
    let mut position = if position_spec == "startpos" {
        Variant::TerachessII.starting_position()
    } else if let Some(fen) = position_spec.strip_prefix("fen ") {
        Position::from_fen(Variant::TerachessII.rules(), fen)
            .map_err(|error| format!("invalid Terachess II FEN: {error}"))?
    } else {
        return Err("position requires `startpos` or `fen ...`".to_owned());
    };
    let mut history = SearchHistory::new(&position);
    let mut played_moves = Vec::new();
    if let Some(moves) = moves_spec {
        for value in moves.split_whitespace() {
            let chess_move = position
                .play_uci(value)
                .map_err(|error| format!("invalid move `{value}` in position command: {error}"))?;
            played_moves.push(chess_move);
            history.record(&position);
        }
    }
    state.position = position;
    state.history = history;
    state.moves = played_moves;
    Ok(())
}

fn go_nodes(command: &str) -> Option<u64> {
    let words = command.split_whitespace().collect::<Vec<_>>();
    words
        .windows(2)
        .find(|window| window[0] == "nodes")
        .and_then(|window| window[1].parse().ok())
}

fn parse<T: std::str::FromStr>(value: &str, name: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for {name}"))
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_command_reconstructs_history() {
        let mut state = State::default();
        set_position(&mut state, "startpos moves d2g5 d15g12").unwrap();
        assert_eq!(
            state.position.side_to_move(),
            capablanca_chess_plus::Color::White
        );
    }

    #[test]
    fn go_nodes_is_an_honest_search_budget() {
        assert_eq!(go_nodes("go nodes 75000"), Some(75_000));
        assert_eq!(go_nodes("go movetime 1000"), None);
    }
}
