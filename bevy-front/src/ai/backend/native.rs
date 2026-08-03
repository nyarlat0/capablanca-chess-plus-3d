use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use super::BackendEvent;

const ENGINE_ENVIRONMENT_VARIABLE: &str = "FAIRY_STOCKFISH_PATH";

pub(crate) struct Backend {
    commands: Sender<String>,
    events: Receiver<BackendEvent>,
}

impl Backend {
    pub(crate) fn new() -> Result<Self, String> {
        let engine_path = resolve_engine_path();
        if !engine_path.is_file() {
            return Err(format!(
                "Fairy-Stockfish executable was not found at {}. Set {ENGINE_ENVIRONMENT_VARIABLE} to override it.",
                engine_path.display()
            ));
        }

        let variants_path = resolve_bundled_file("variants.ini");
        if !variants_path.is_file() {
            return Err(format!(
                "Fairy-Stockfish variant configuration was not found at {}.",
                variants_path.display()
            ));
        }

        let (command_sender, command_receiver) = mpsc::channel::<String>();
        let (event_sender, event_receiver) = mpsc::channel::<BackendEvent>();
        thread::Builder::new()
            .name("fairy-stockfish".to_owned())
            .spawn(move || {
                run_engine(&engine_path, &variants_path, command_receiver, event_sender);
            })
            .map_err(|error| format!("could not start the Fairy-Stockfish host thread: {error}"))?;

        Ok(Self {
            commands: command_sender,
            events: event_receiver,
        })
    }

    pub(crate) fn send(&self, command: &str) -> Result<(), String> {
        self.commands
            .send(command.to_owned())
            .map_err(|_| "the Fairy-Stockfish process is no longer running".to_owned())
    }

    pub(crate) fn drain(&self, destination: &mut Vec<BackendEvent>) {
        while let Ok(event) = self.events.try_recv() {
            destination.push(event);
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.commands.send("quit".to_owned());
    }
}

fn resolve_engine_path() -> PathBuf {
    env::var_os(ENGINE_ENVIRONMENT_VARIABLE)
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_bundled_file("fairy-stockfish"))
}

fn resolve_bundled_file(file_name: &str) -> PathBuf {
    let mut directories = Vec::new();
    if let Ok(executable) = env::current_exe()
        && let Some(directory) = executable.parent()
    {
        directories.push(directory.join("assets").join("engine"));
    }
    if let Ok(directory) = env::current_dir() {
        directories.push(directory.join("assets").join("engine"));
        directories.push(directory.join("bevy-front").join("assets").join("engine"));
    }
    directories.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("engine"),
    );

    directories
        .iter()
        .map(|directory| directory.join(file_name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            directories
                .last()
                .expect("at least the manifest asset directory is available")
                .join(file_name)
        })
}

fn run_engine(
    engine_path: &Path,
    variants_path: &Path,
    command_receiver: Receiver<String>,
    event_sender: Sender<BackendEvent>,
) {
    let mut child = match Command::new(engine_path)
        .arg("load")
        .arg(variants_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = event_sender.send(BackendEvent::Error(format!(
                "could not launch {}: {error}",
                engine_path.display()
            )));
            return;
        }
    };

    let Some(mut stdin) = child.stdin.take() else {
        let _ = event_sender.send(BackendEvent::Error(
            "Fairy-Stockfish did not expose its standard input".to_owned(),
        ));
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = event_sender.send(BackendEvent::Error(
            "Fairy-Stockfish did not expose its standard output".to_owned(),
        ));
        return;
    };
    let stderr = child.stderr.take();

    let writer_events = event_sender.clone();
    thread::spawn(move || {
        for command in command_receiver {
            if writeln!(stdin, "{command}")
                .and_then(|()| stdin.flush())
                .is_err()
            {
                let _ = writer_events.send(BackendEvent::Error(
                    "failed to write to Fairy-Stockfish".to_owned(),
                ));
                break;
            }
            if command == "quit" {
                break;
            }
        }
    });

    if let Some(stderr) = stderr {
        let stderr_events = event_sender.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = stderr_events.send(BackendEvent::Error(format!("Fairy-Stockfish: {line}")));
            }
        });
    }

    for line in BufReader::new(stdout).lines() {
        match line {
            Ok(line) => {
                if event_sender.send(BackendEvent::Line(line)).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = event_sender.send(BackendEvent::Error(format!(
                    "failed to read Fairy-Stockfish output: {error}"
                )));
                return;
            }
        }
    }

    match child.wait() {
        Ok(status) if status.success() => {}
        Ok(status) => {
            let _ = event_sender.send(BackendEvent::Error(format!(
                "Fairy-Stockfish exited with {status}"
            )));
        }
        Err(error) => {
            let _ = event_sender.send(BackendEvent::Error(format!(
                "could not wait for Fairy-Stockfish: {error}"
            )));
        }
    }
}
