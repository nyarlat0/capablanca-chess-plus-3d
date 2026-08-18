use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use capablanca_chess_plus::{Position, Variant};
use terastockfish::{
    AnalysisInfo, SearchControl, SearchLimits, SearchOptions, Searcher, mate_distance,
};

use super::BackendEvent;

const DEFAULT_HASH_MEGABYTES: usize = 16;

pub(crate) struct Backend {
    commands: Sender<String>,
    events: Receiver<BackendEvent>,
    worker: Option<JoinHandle<()>>,
}

impl Backend {
    pub(crate) fn new() -> Result<Self, String> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("terastockfish-frontend".to_owned())
            .spawn(move || run_engine(command_receiver, event_sender))
            .map_err(|error| format!("could not start TeraStockfish: {error}"))?;
        Ok(Self {
            commands: command_sender,
            events: event_receiver,
            worker: Some(worker),
        })
    }

    pub(crate) fn send(&self, command: &str) -> Result<(), String> {
        self.commands
            .send(command.to_owned())
            .map_err(|_| "TeraStockfish is no longer running".to_owned())
    }

    pub(crate) fn drain(&self, destination: &mut Vec<BackendEvent>) {
        destination.extend(self.events.try_iter());
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.commands.send("quit".to_owned());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct ActiveSearch {
    control: SearchControl,
    worker: JoinHandle<Searcher>,
}

fn run_engine(commands: Receiver<String>, events: Sender<BackendEvent>) {
    let mut position = Variant::TerachessII.starting_position();
    let mut searcher = Some(new_searcher(DEFAULT_HASH_MEGABYTES));
    let mut active = None;
    loop {
        reap_search(&mut active, &mut searcher, &events, false);
        match commands.recv_timeout(Duration::from_millis(5)) {
            Ok(command) if command == "quit" => {
                reap_search(&mut active, &mut searcher, &events, true);
                break;
            }
            Ok(command) => {
                if let Err(message) =
                    handle_command(&command, &events, &mut position, &mut searcher, &mut active)
                {
                    let _ = events.send(BackendEvent::Error(message));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                reap_search(&mut active, &mut searcher, &events, true);
                break;
            }
        }
    }
}

fn new_searcher(hash_megabytes: usize) -> Searcher {
    Searcher::new(SearchOptions {
        hash_megabytes,
        threads: 1,
    })
}

fn handle_command(
    command: &str,
    events: &Sender<BackendEvent>,
    position: &mut Position,
    searcher: &mut Option<Searcher>,
    active: &mut Option<ActiveSearch>,
) -> Result<(), String> {
    if command == "uci" {
        for line in [
            "id name TeraStockfish Native",
            "option name Hash type spin default 16 min 1 max 256",
            "option name Threads type spin default 1 min 1 max 1",
            "option name UCI_Variant type combo default terachessii var terachessii",
            "uciok",
        ] {
            send_line(events, line);
        }
        Ok(())
    } else if command == "isready" {
        send_line(events, "readyok");
        Ok(())
    } else if command == "ucinewgame" {
        reap_search(active, searcher, events, true);
        *position = Variant::TerachessII.starting_position();
        searcher
            .as_ref()
            .expect("an idle TeraStockfish searcher exists")
            .clear_hash();
        Ok(())
    } else if command == "stop" {
        reap_search(active, searcher, events, true);
        Ok(())
    } else if let Some(value) = command.strip_prefix("setoption name Hash value ") {
        reap_search(active, searcher, events, true);
        let megabytes = value
            .parse::<usize>()
            .map_err(|_| "Hash must be an integer".to_owned())?
            .clamp(1, 256);
        searcher
            .as_mut()
            .expect("an idle TeraStockfish searcher exists")
            .resize_hash(megabytes);
        Ok(())
    } else if command.starts_with("setoption name Threads value ") {
        Ok(())
    } else if let Some(value) = command.strip_prefix("setoption name UCI_Variant value ") {
        if matches!(
            value.to_ascii_lowercase().as_str(),
            "terachessii" | "terachess ii"
        ) {
            Ok(())
        } else {
            Err(format!("unsupported TeraStockfish variant: {value}"))
        }
    } else if let Some(fen) = command.strip_prefix("position fen ") {
        reap_search(active, searcher, events, true);
        *position = Position::from_fen(Variant::TerachessII.rules(), fen)
            .map_err(|error| error.to_string())?;
        Ok(())
    } else if let Some(value) = command.strip_prefix("go nodes ") {
        reap_search(active, searcher, events, true);
        let nodes = value
            .parse::<u64>()
            .map_err(|_| "node budget must be an integer".to_owned())?
            .max(1);
        start_search(position.clone(), nodes, searcher, active, events);
        Ok(())
    } else {
        Err(format!("unknown TeraStockfish command: {command}"))
    }
}

fn start_search(
    position: Position,
    nodes: u64,
    searcher: &mut Option<Searcher>,
    active: &mut Option<ActiveSearch>,
    events: &Sender<BackendEvent>,
) {
    let mut engine = searcher
        .take()
        .expect("a new search starts only while TeraStockfish is idle");
    let control = engine.control();
    let output = events.clone();
    let worker = thread::spawn(move || {
        let info_output = output.clone();
        let result = engine.analyze_with(
            &position,
            SearchLimits {
                max_depth: 191,
                max_nodes: Some(nodes),
                ..SearchLimits::default()
            },
            move |info| send_info(&info_output, info),
        );
        let best_move = result
            .best_move
            .map_or_else(|| "0000".to_owned(), |chess_move| chess_move.to_uci());
        send_line(&output, &format!("bestmove {best_move}"));
        engine
    });
    *active = Some(ActiveSearch { control, worker });
}

fn reap_search(
    active: &mut Option<ActiveSearch>,
    searcher: &mut Option<Searcher>,
    events: &Sender<BackendEvent>,
    stop: bool,
) {
    let should_join = active
        .as_ref()
        .is_some_and(|search| stop || search.worker.is_finished());
    if !should_join {
        return;
    }
    let search = active
        .take()
        .expect("join was requested for an active search");
    if stop {
        search.control.stop();
    }
    match search.worker.join() {
        Ok(engine) => *searcher = Some(engine),
        Err(_) => {
            *searcher = Some(new_searcher(DEFAULT_HASH_MEGABYTES));
            let _ = events.send(BackendEvent::Error(
                "TeraStockfish search worker panicked".to_owned(),
            ));
        }
    }
}

fn send_info(events: &Sender<BackendEvent>, info: &AnalysisInfo) {
    let score = mate_distance(info.score).map_or_else(
        || format!("cp {}", info.score),
        |distance| format!("mate {distance}"),
    );
    send_line(
        events,
        &format!(
            "info depth {} score {} nodes {}",
            info.depth, score, info.nodes
        ),
    );
}

fn send_line(events: &Sender<BackendEvent>, line: &str) {
    let _ = events.send(BackendEvent::Line(line.to_owned()));
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn wait_for_line(backend: &Backend, predicate: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut events = Vec::new();
            backend.drain(&mut events);
            for event in events {
                match event {
                    BackendEvent::Line(line) if predicate(&line) => return line,
                    BackendEvent::Line(_) => {}
                    BackendEvent::Error(error) => panic!("TeraStockfish failed: {error}"),
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for TeraStockfish"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn frontend_backend_returns_a_legal_terachess_move() {
        let backend = Backend::new().unwrap();
        backend.send("uci").unwrap();
        wait_for_line(&backend, |line| line == "uciok");
        let position = Variant::TerachessII.starting_position();
        backend
            .send(&format!("position fen {}", position.to_fen()))
            .unwrap();
        backend.send("go nodes 1000").unwrap();
        let reply = wait_for_line(&backend, |line| line.starts_with("bestmove "));
        let best_move = reply.split_whitespace().nth(1).unwrap();
        position.parse_uci_move(best_move).unwrap();
    }
}
