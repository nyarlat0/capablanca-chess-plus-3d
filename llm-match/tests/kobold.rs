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
impl Server {
    fn new(answers: Vec<&'static str>) -> Self {
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
                let response = match path.as_str() {
                    "/api/extra/true_max_context_length" | "/api/v1/config/max_context_length" => json!({"value":16384}),
                    "/api/extra/tokencount" => json!({"value":512}),
                    "/api/v1/generate" => {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        recorded.lock().unwrap().push(request["prompt"].as_str().unwrap().into());
                        json!({"results":[{"text":answers.next().expect("unexpected extra generation")}]})
                    }
                    _ => panic!("unexpected endpoint {path}"),
                }.to_string();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
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
    assert_eq!(rejected.len(), 2);
    assert_eq!(state.game().position().to_fen(), fen);
    assert_eq!(state.history().len(), 1);
    assert_eq!(state.wrong_moves().len(), 2);
    let mv = llm_match::generate_move(&mut state, &config, |_, _| panic!("valid answer rejected"))
        .await
        .unwrap();
    assert_eq!(mv.to_uci(), "e7e5");
    assert_eq!(state.history().len(), 2);
    assert!(state.wrong_moves().is_empty());
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 3);
    for prompt in prompts.iter() {
        assert!(prompt.contains(&fen));
    }
    assert!(!prompts[0].contains("Rejected illegal"));
    assert!(prompts[1].contains("I choose e7e5"));
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
