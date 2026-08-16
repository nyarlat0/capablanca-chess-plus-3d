//! Minimal, standards-oriented UCI frontend for GUI and command-line use.

use crate::search::mate_distance;
use crate::{AnalysisInfo, SearchLimits, Searcher, evaluate};
use capablanca_chess_plus::{Color, Position, Variant};
use std::io::{self, BufRead, Write};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const ENGINE_NAME: &str = "TeraStockfish 0.1";
const MAX_UCI_DEPTH: u8 = 191;

struct ActiveSearch {
    control: crate::SearchControl,
    thread: JoinHandle<Searcher>,
}

struct UciState {
    position: Position,
    searcher: Option<Searcher>,
    active: Option<ActiveSearch>,
}

impl Default for UciState {
    fn default() -> Self {
        Self {
            position: Variant::TerachessII.starting_position(),
            searcher: Some(Searcher::default()),
            active: None,
        }
    }
}

impl UciState {
    fn reap_search(&mut self) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.thread.is_finished())
        {
            self.join_search(false);
        }
    }

    fn join_search(&mut self, stop: bool) {
        let Some(active) = self.active.take() else {
            return;
        };
        if stop {
            active.control.stop();
        }
        match active.thread.join() {
            Ok(searcher) => self.searcher = Some(searcher),
            Err(_) => {
                eprintln!("info string search worker panicked; rebuilding search state");
                self.searcher = Some(Searcher::default());
            }
        }
    }

    fn start_search(&mut self, limits: SearchLimits) {
        self.join_search(true);
        let mut searcher = self.searcher.take().unwrap_or_default();
        let control = searcher.control();
        let position = self.position.clone();
        let worker = thread::spawn(move || {
            let result = searcher.analyze_with(&position, limits, print_info);
            let best_move = result
                .best_move
                .map_or_else(|| "0000".to_owned(), |chess_move| chess_move.to_uci());
            println!("bestmove {best_move}");
            let _ = io::stdout().flush();
            searcher
        });
        self.active = Some(ActiveSearch {
            control,
            thread: worker,
        });
    }

    fn replace_position(&mut self, command: &str) -> Result<(), String> {
        self.join_search(true);
        self.position = parse_position(command)?;
        Ok(())
    }

    fn set_option(&mut self, command: &str) -> Result<(), String> {
        self.join_search(true);
        let (name, value) = parse_setoption(command)?;
        let searcher = self.searcher.as_mut().expect("idle searcher must exist");
        match name.to_ascii_lowercase().as_str() {
            "hash" => {
                let megabytes = value
                    .ok_or_else(|| "Hash requires a value".to_owned())?
                    .parse::<usize>()
                    .map_err(|_| "Hash must be an integer".to_owned())?
                    .clamp(1, 65_536);
                searcher.resize_hash(megabytes);
            }
            "threads" => {
                let threads = value
                    .ok_or_else(|| "Threads requires a value".to_owned())?
                    .parse::<usize>()
                    .map_err(|_| "Threads must be an integer".to_owned())?
                    .clamp(1, 256);
                searcher.set_threads(threads);
            }
            "clear hash" => searcher.clear_hash(),
            "uci_variant" => {
                let variant = value.unwrap_or_default().to_ascii_lowercase();
                if !matches!(variant.as_str(), "terachessii" | "terachess ii") {
                    return Err(format!("unsupported UCI_Variant: {variant}"));
                }
            }
            _ => return Err(format!("unknown option: {name}")),
        }
        Ok(())
    }
}

/// Runs the engine protocol over stdin/stdout until `quit` or EOF.
pub fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let mut state = UciState::default();
    for line in stdin.lock().lines() {
        let line = line?;
        let command = line.trim();
        state.reap_search();
        if command.is_empty() {
            continue;
        }
        let result = if command == "uci" {
            print_identification();
            Ok(())
        } else if command == "isready" {
            println!("readyok");
            Ok(())
        } else if command == "ucinewgame" {
            state.join_search(true);
            state.position = Variant::TerachessII.starting_position();
            state
                .searcher
                .as_ref()
                .expect("idle searcher must exist")
                .clear_hash();
            Ok(())
        } else if command.starts_with("setoption ") {
            state.set_option(command)
        } else if command.starts_with("position ") {
            state.replace_position(command)
        } else if command == "stop" {
            state.join_search(true);
            Ok(())
        } else if command == "quit" {
            state.join_search(true);
            break;
        } else if command == "d" {
            println!("Fen: {}", state.position.to_fen());
            Ok(())
        } else if command == "eval" {
            println!(
                "info string static evaluation {} cp",
                evaluate(&state.position)
            );
            Ok(())
        } else if command == "go" || command.starts_with("go ") {
            match parse_go(command, state.position.side_to_move()) {
                Ok(GoCommand::Search(limits)) => {
                    state.start_search(limits);
                    Ok(())
                }
                Ok(GoCommand::Perft(depth)) => {
                    state.join_search(true);
                    let nodes = state.position.perft(depth);
                    println!("info string perft {depth} nodes {nodes}");
                    println!("bestmove 0000");
                    Ok(())
                }
                Err(error) => Err(error),
            }
        } else {
            Err(format!("unknown command: {command}"))
        };
        if let Err(error) = result {
            println!("info string error: {error}");
        }
        io::stdout().flush()?;
    }
    state.join_search(true);
    Ok(())
}

fn print_identification() {
    println!("id name {ENGINE_NAME}");
    println!("id author Capablanca Chess Plus contributors");
    println!("option name Hash type spin default 128 min 1 max 65536");
    println!("option name Threads type spin default 1 min 1 max 256");
    println!("option name Clear Hash type button");
    println!("option name UCI_Variant type combo default terachessii var terachessii");
    println!("uciok");
}

fn print_info(info: &AnalysisInfo) {
    let score = if let Some(mate) = mate_distance(info.score) {
        format!("mate {mate}")
    } else {
        format!("cp {}", info.score)
    };
    let time = info.elapsed.as_millis();
    let pv = info
        .principal_variation
        .iter()
        .map(|chess_move| chess_move.to_uci())
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "info depth {} score {} nodes {} nps {} hashfull {} time {} pv {}",
        info.depth, score, info.nodes, info.nps, info.hashfull, time, pv
    );
}

fn parse_position(command: &str) -> Result<Position, String> {
    let words = command.split_whitespace().collect::<Vec<_>>();
    let mut cursor = 1;
    let mut position = match words.get(cursor).copied() {
        Some("startpos") => {
            cursor += 1;
            Variant::TerachessII.starting_position()
        }
        Some("fen") => {
            cursor += 1;
            if words.len() < cursor + 6 {
                return Err("position fen requires all six FEN fields".to_owned());
            }
            let fen = words[cursor..cursor + 6].join(" ");
            cursor += 6;
            Position::from_fen(Variant::TerachessII.rules(), &fen)
                .map_err(|error| error.to_string())?
        }
        _ => return Err("position must contain startpos or fen".to_owned()),
    };
    if words.get(cursor) == Some(&"moves") {
        cursor += 1;
    }
    for chess_move in &words[cursor..] {
        position
            .play_uci(chess_move)
            .map_err(|error| error.to_string())?;
    }
    Ok(position)
}

fn parse_setoption(command: &str) -> Result<(String, Option<String>), String> {
    let words = command.split_whitespace().collect::<Vec<_>>();
    if words.get(1) != Some(&"name") {
        return Err("setoption requires `name`".to_owned());
    }
    let value_at = words.iter().position(|word| *word == "value");
    let name_end = value_at.unwrap_or(words.len());
    if name_end <= 2 {
        return Err("setoption name cannot be empty".to_owned());
    }
    let name = words[2..name_end].join(" ");
    let value = value_at.map(|index| words[index + 1..].join(" "));
    Ok((name, value))
}

enum GoCommand {
    Search(SearchLimits),
    Perft(u8),
}

fn parse_go(command: &str, side: Color) -> Result<GoCommand, String> {
    let words = command.split_whitespace().collect::<Vec<_>>();
    let mut limits = SearchLimits {
        max_depth: 64,
        ..SearchLimits::default()
    };
    let mut white_time = None;
    let mut black_time = None;
    let mut white_increment = Duration::ZERO;
    let mut black_increment = Duration::ZERO;
    let mut cursor = 1;
    while cursor < words.len() {
        let word = words[cursor];
        if word == "infinite" {
            limits.infinite = true;
            limits.max_depth = MAX_UCI_DEPTH;
            cursor += 1;
            continue;
        }
        let value = words
            .get(cursor + 1)
            .ok_or_else(|| format!("go {word} requires a value"))?;
        match word {
            "depth" => limits.max_depth = parse_u8(value, "depth")?.clamp(1, MAX_UCI_DEPTH),
            "nodes" => {
                limits.max_nodes = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| "nodes must be an integer".to_owned())?
                        .max(1),
                );
            }
            "movetime" => limits.move_time = Some(parse_millis(value, "movetime")?),
            "wtime" => white_time = Some(parse_millis(value, "wtime")?),
            "btime" => black_time = Some(parse_millis(value, "btime")?),
            "winc" => white_increment = parse_millis(value, "winc")?,
            "binc" => black_increment = parse_millis(value, "binc")?,
            "movestogo" => {
                limits.moves_to_go = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| "movestogo must be an integer".to_owned())?
                        .max(1),
                );
            }
            "perft" => return Ok(GoCommand::Perft(parse_u8(value, "perft")?)),
            "mate" | "ponder" | "searchmoves" => {
                return Err(format!("go {word} is not supported yet"));
            }
            _ => return Err(format!("unknown go parameter: {word}")),
        }
        cursor += 2;
    }
    if limits.move_time.is_none() {
        limits.remaining_time = match side {
            Color::White => white_time,
            Color::Black => black_time,
        };
        limits.increment = match side {
            Color::White => white_increment,
            Color::Black => black_increment,
        };
    }
    Ok(GoCommand::Search(limits))
}

fn parse_u8(value: &str, name: &str) -> Result<u8, String> {
    value
        .parse::<u8>()
        .map_err(|_| format!("{name} must be an integer"))
}

fn parse_millis(value: &str, name: &str) -> Result<Duration, String> {
    value
        .parse::<u64>()
        .map(Duration::from_millis)
        .map_err(|_| format!("{name} must be an integer number of milliseconds"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_command_replays_large_board_moves() {
        let mut expected = Variant::TerachessII.starting_position();
        expected.play_uci("a4a6").unwrap();
        let parsed = parse_position("position startpos moves a4a6").unwrap();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn go_uses_the_clock_of_the_moving_side() {
        let GoCommand::Search(limits) =
            parse_go("go wtime 12000 btime 8000 winc 100", Color::Black).unwrap()
        else {
            panic!("expected a search");
        };
        assert_eq!(limits.remaining_time, Some(Duration::from_secs(8)));
        assert_eq!(limits.increment, Duration::ZERO);
    }

    #[test]
    fn setoption_parser_accepts_names_with_spaces() {
        assert_eq!(
            parse_setoption("setoption name Clear Hash").unwrap(),
            ("Clear Hash".to_owned(), None)
        );
    }
}
