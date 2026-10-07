use capablanca_chess_plus::{Color, Variant};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Server {
    endpoint: String,
    prompts: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
enum Reply {
    Text(&'static str),
    Http(u16),
    Slow(&'static str),
}
impl Server {
    fn new(answers: Vec<&'static str>) -> Self {
        Self::script(answers.into_iter().map(Reply::Text).collect(), 512)
    }
    fn script(answers: Vec<Reply>, token_count: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let recorded = prompts.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = thread::spawn(move || {
            let mut answers = answers.into_iter();
            while !stopping.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(client) => client,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&socket);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut status = 200;
                let response = match path.as_str() {
                    "/api/extra/true_max_context_length" | "/api/v1/config/max_context_length" => {
                        json!({"value":16384})
                    }
                    "/api/extra/tokencount" => json!({"value":token_count}),
                    "/api/v1/generate" => {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        recorded
                            .lock()
                            .unwrap()
                            .push(request["prompt"].as_str().unwrap().into());
                        match answers.next().expect("unexpected extra generation") {
                            Reply::Text(text) => json!({"results":[{"text":text}]}),
                            Reply::Http(code) => {
                                status = code;
                                json!({"error":"test error"})
                            }
                            Reply::Slow(text) => {
                                thread::sleep(Duration::from_millis(1200));
                                json!({"results":[{"text":text}]})
                            }
                        }
                    }
                    _ => panic!("unexpected endpoint {path}"),
                }
                .to_string();
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
            }
        });
        Self {
            endpoint,
            prompts,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[tokio::test]
async fn illegal_retries_keep_history_and_board_then_commit_only_valid_answer() {
    let server = Server::new(vec!["I choose e7e5", "e7e4", " e7e5\n"]);
    let mut config = llm_match::Config::load().unwrap();
    config.endpoint = server.endpoint.clone();
    config.max_attempts = 2;
    let mut state = llm_match::Match::new(Variant::Gothic, Color::Black);
    state.accept_human("e2e4").unwrap();
    let fen = state.game().position().to_fen();
    let mut rejected = Vec::new();
    assert!(
        llm_match::generate_move(&mut state, &config, |_, raw| rejected.push(raw.to_owned()))
            .await
            .is_err()
    );
    assert_eq!(rejected, ["e7e4"]);
    assert_eq!(state.game().position().to_fen(), fen);
    assert_eq!(state.history().len(), 1);
    assert_eq!(state.wrong_moves(), ["e7e4"]);
    let mv = llm_match::generate_move(&mut state, &config, |_, _| panic!("valid answer rejected"))
        .await
        .unwrap();
    assert_eq!(mv.to_uci(), "e7e5");
    assert_eq!(state.history().len(), 2);
    assert!(state.wrong_moves().is_empty());
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 3);
    for prompt in prompts.iter() {
        assert!(!prompt.contains(&fen));
        assert!(prompt.contains("White Pawn e2-e4"));
    }
    assert_eq!(prompts[0], prompts[1]);
    assert!(!prompts[0].contains("Rejected moves"));
    assert!(!prompts[1].contains("I choose e7e5"));
    assert!(prompts[2].contains("e7e4"));
    let history0 = prompts[0]
        .split("AUTHORITATIVE CURRENT POSITION")
        .next()
        .unwrap();
    for prompt in prompts.iter().skip(1) {
        assert_eq!(
            prompt
                .split("AUTHORITATIVE CURRENT POSITION")
                .next()
                .unwrap(),
            history0
        );
    }
}

fn configured(server: &Server, attempts: usize) -> llm_match::Config {
    let mut config = llm_match::Config::load().unwrap();
    config.endpoint = server.endpoint.clone();
    config.max_attempts = attempts;
    config.reasoning = prompt_core::ReasoningTemplate {
        name: "test".into(),
        prefix: "<thinking>".into(),
        suffix: "</thinking>".into(),
        separator: "\n".into(),
    };
    config
}
fn black_turn() -> llm_match::Match {
    let mut state = llm_match::Match::new(Variant::Gothic, Color::Black);
    state.accept_human("e2e4").unwrap();
    state
}

#[tokio::test]
async fn truncated_and_empty_reasoning_retry_silently_without_changing_context() {
    let server = Server::new(vec![
        "<thinking>private unclosed f2f4",
        "<thinking>private complete</thinking>  ",
        "<thinking>private</thinking>e7e5",
    ]);
    let mut state = black_turn();
    let original = state.game().position().to_fen();
    // Seed a genuine prior rejected move: silent retries must preserve it too.
    assert!(state.accept_model("e7e4").is_err());
    let config = configured(&server, 2);
    assert!(
        llm_match::generate_move(&mut state, &config, |_, _| panic!(
            "silent retry emitted rejected"
        ))
        .await
        .is_err()
    );
    assert_eq!(state.game().position().to_fen(), original);
    assert_eq!(state.uci_history(), ["e2e4"]);
    assert_eq!(state.history()[0].content, "White Pawn e2-e4");
    assert_eq!(state.wrong_moves(), ["e7e4"]);
    llm_match::generate_move(&mut state, &config, |_, _| panic!("unexpected rejection"))
        .await
        .unwrap();
    assert_eq!(state.uci_history(), ["e2e4", "e7e5"]);
    assert!(state.wrong_moves().is_empty());
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 3);
    assert!(prompts.windows(2).all(|p| p[0] == p[1]));
    assert!(
        prompts
            .iter()
            .all(|p| !p.contains("private") && !p.contains("truncated"))
    );
}

#[tokio::test]
async fn only_the_rejected_final_move_is_visible() {
    let server = Server::new(vec![
        "<thinking>secret reasoning suggests other moves</thinking>e7e4",
        "<thinking>secret again</thinking>e7e5",
    ]);
    let mut state = black_turn();
    let mut events = Vec::new();
    let config = configured(&server, 1);
    assert!(
        llm_match::generate_move(&mut state, &config, |_, text| events.push(text.to_owned()))
            .await
            .is_err()
    );
    assert_eq!(state.wrong_moves(), ["e7e4"]);
    assert_eq!(events, ["e7e4"]);
    llm_match::generate_move(&mut state, &config, |_, _| panic!())
        .await
        .unwrap();
    let prompts = server.prompts.lock().unwrap();
    assert!(
        prompts[1].contains("Rejected moves: [\"e7e4\"]. Position unchanged. Do not repeat them.")
    );
    assert!(prompts.iter().all(|p| !p.contains("secret")));
}

#[tokio::test]
async fn transient_http_failures_retry_without_feedback() {
    let server = Server::script(
        vec![
            Reply::Http(408),
            Reply::Http(429),
            Reply::Http(503),
            Reply::Text("e7e5"),
        ],
        512,
    );
    let mut state = black_turn();
    llm_match::generate_move(&mut state, &configured(&server, 4), |_, _| {
        panic!("transport rejection")
    })
    .await
    .unwrap();
    assert_eq!(state.uci_history(), ["e2e4", "e7e5"]);
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 4);
    assert!(prompts.windows(2).all(|p| p[0] == p[1]));
}

#[tokio::test]
async fn timeout_retries_without_feedback() {
    let server = Server::script(vec![Reply::Slow("e7e4"), Reply::Text("e7e5")], 512);
    let mut config = configured(&server, 2);
    config.timeout_seconds = 1;
    let mut state = black_turn();
    llm_match::generate_move(&mut state, &config, |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(state.uci_history(), ["e2e4", "e7e5"]);
    assert!(state.wrong_moves().is_empty());
}

#[tokio::test]
async fn connect_failures_exhaust_retry_budget_without_state_changes() {
    let mut config = llm_match::Config::load().unwrap();
    config.endpoint = "http://127.0.0.1:0".into();
    config.max_attempts = 3;
    config.timeout_seconds = 1;
    let mut state = black_turn();
    let before = state.game().position().to_fen();
    let error = llm_match::generate_move(&mut state, &config, |_, _| panic!())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("after 3 attempts"), "{error:#}");
    assert_eq!(state.uci_history(), ["e2e4"]);
    assert_eq!(state.game().position().to_fen(), before);
    assert!(state.wrong_moves().is_empty());
}

#[tokio::test]
async fn deterministic_http_and_context_errors_fail_without_generation_retries() {
    for code in [400, 401, 403, 404, 422] {
        let server = Server::script(vec![Reply::Http(code)], 512);
        let mut state = black_turn();
        assert!(
            llm_match::generate_move(&mut state, &configured(&server, 3), |_, _| panic!())
                .await
                .is_err()
        );
        assert_eq!(server.prompts.lock().unwrap().len(), 1);
        assert_eq!(state.uci_history(), ["e2e4"]);
        assert!(state.wrong_moves().is_empty());
    }
    let server = Server::script(vec![], 100_000);
    let mut state = black_turn();
    assert!(
        llm_match::generate_move(&mut state, &configured(&server, 3), |_, _| panic!())
            .await
            .is_err()
    );
    assert!(server.prompts.lock().unwrap().is_empty());
    assert_eq!(state.uci_history(), ["e2e4"]);
}
