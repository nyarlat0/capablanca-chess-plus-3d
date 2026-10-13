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
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
enum Reply {
    Text(String),
    Http(u16),
    Slow(&'static str),
}
impl Server {
    fn script(answers: Vec<Reply>, token_count: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let recorded = prompts.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded_requests = requests.clone();
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
                    "/api/extra/tokencount" => {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        let prompt = request["messages"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| m["content"].as_str().unwrap())
                            .collect::<Vec<_>>()
                            .join("\n");
                        json!({"value":token_count, "prompt":prompt})
                    }
                    "/v1/chat/completions" => {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        assert!(request.get("prompt").is_none());
                        let prompt = request["messages"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| m["content"].as_str().unwrap())
                            .collect::<Vec<_>>()
                            .join("\n");
                        recorded.lock().unwrap().push(prompt);
                        recorded_requests.lock().unwrap().push(request);
                        match answers.next().expect("unexpected extra generation") {
                            Reply::Text(text) => {
                                json!({"choices":[{"message":{"role":"assistant","content":text}}]})
                            }
                            Reply::Http(code) => {
                                status = code;
                                json!({"error":"test error"})
                            }
                            Reply::Slow(text) => {
                                thread::sleep(Duration::from_millis(1200));
                                json!({"choices":[{"message":{"role":"assistant","content":text}}]})
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
            requests,
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

fn text(value: Value) -> Reply {
    Reply::Text(value.to_string())
}
fn analysis() -> Reply {
    Reply::Text(format!(
        "<thinking>PRIVATE_THINKING</thinking>## ANALYST_PRIVATE_FACT\n- {}\nAny format, including {{braces}}, is fine.",
        "A free-form observation. ".repeat(20)
    ))
}
fn candidates(moves: &[(&str, &str, &str)]) -> Reply {
    text(
        json!({"candidates": moves.iter().enumerate().map(|(i,(mv,from,to))|
        json!({"id":format!("c{}",i+1),"move":mv,"from":from,"to":to,"rationale":"PROPOSER_PRIVATE_RATIONALE"})).collect::<Vec<_>>()}),
    )
}
fn verify(id: usize, accept: bool) -> Reply {
    text(
        json!({"candidate_id":format!("c{id}"),"verdict":if accept {"ACCEPT"} else {"REJECT"},"reason":format!("VERIFIER_NOTE_{id}")}),
    )
}
fn select(id: usize, mv: &str) -> Reply {
    text(json!({"candidate_id":format!("c{id}"),"final_move":mv}))
}
fn normal_proposal() -> Reply {
    candidates(&[("e5", "e7", "e5"), ("d5", "d7", "d5"), ("Nf6", "g8", "f6")])
}
fn cfg(server: &Server) -> llm_match::Config {
    let mut c = llm_match::Config::load_classic_with_profile(Some("with-history")).unwrap();
    c.endpoint = server.endpoint.clone();
    c.stage_max_attempts = 3;
    c.max_attempts = 2;
    c.reasoning = prompt_core::ReasoningTemplate {
        name: "test".into(),
        prefix: "<thinking>".into(),
        suffix: "</thinking>".into(),
        separator: "\n".into(),
    };
    c
}
fn state() -> llm_match::Match {
    let mut m = llm_match::Match::new(Variant::Classic, Color::Black);
    m.accept_human("e2e4").unwrap();
    m
}
#[tokio::test]
async fn structured_stages_accept_markdown_json_after_reasoning() {
    let fenced = |reply| match reply {
        Reply::Text(body) => Reply::Text(format!(
            "<thinking>private</thinking>\n```json\n{body}\n```"
        )),
        _ => unreachable!(),
    };
    let server = Server::script(
        vec![
            analysis(),
            fenced(normal_proposal()),
            fenced(verify(1, true)),
            fenced(verify(2, false)),
            fenced(verify(3, false)),
            fenced(select(1, "e5")),
        ],
        512,
    );
    let mut m = state();
    llm_match::generate_move(&mut m, &cfg(&server), |_, _| panic!("unexpected rejection"))
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
    assert_eq!(server.prompts.lock().unwrap().len(), 6);
}

#[tokio::test]
async fn independent_stages_commit_only_selected_move() {
    let server = Server::script(
        vec![
            analysis(),
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, true),
            select(1, "e5"),
        ],
        512,
    );
    let mut m = state();
    let c = cfg(&server);
    llm_match::generate_move(&mut m, &c, |_, _| panic!("unexpected rejection"))
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
    assert_eq!(m.classic_san_history(), "1. e4 e5");
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 6);
    let requests = server.requests.lock().unwrap();
    for request in requests.iter() {
        assert_eq!(request["stream"], false);
        assert_eq!(request["continue_assistant_turn"], false);
        assert!(request["max_tokens"].as_u64().unwrap() > 0);
        assert_eq!(request["stop"], json!([]));
        let messages = request["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages.last().unwrap()["role"], "user");
        assert!(
            messages
                .iter()
                .all(|m| !m["content"].as_str().unwrap().contains("<|im_start|>"))
        );
    }
    assert_eq!(
        requests[0]["messages"][1],
        json!({"role":"user","content":"e4"})
    );
    assert_eq!(requests[2]["messages"].as_array().unwrap().len(), 2);
    assert!(p[1].contains("ANALYST_PRIVATE_FACT"));
    assert!(p[1].contains("Any format, including {braces}, is fine."));
    assert!(p.iter().all(|prompt| !prompt.contains("PRIVATE_THINKING")));
    let initial = state().game().position().to_fen();
    for prompt in p.iter() {
        assert!(prompt.contains(&initial));
        assert!(prompt.contains("ASCII BOARD"));
    }
    for prompt in &p[2..] {
        assert!(!prompt.contains("ANALYST_PRIVATE_FACT"));
        assert!(!prompt.contains("PROPOSER_PRIVATE_RATIONALE"));
        assert!(!prompt.contains("1. e4"));
    }
    assert!(p[2].contains("\"move\":\"e5\""));
    assert!(!p[2].contains("\"move\":\"d5\""));
    assert!(!p[2].contains("\"move\":\"Nf6\""));
    assert!(!p[5].contains("\"move\":\"d5\""));
    assert!(!p[5].contains("VERIFIER_NOTE_2"));
    assert!(p[5].contains("VERIFIER_NOTE_1"));
    for mv in state().game().position().legal_moves() {
        for prompt in p.iter() {
            assert!(!prompt.contains(&mv.to_uci()));
        }
    }
}
#[tokio::test]
async fn empty_freeform_analysis_retries_without_feedback() {
    let server = Server::script(
        vec![
            Reply::Text("   ".into()),
            Reply::Text("A plain observation, not JSON.".into()),
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut m = state();
    let mut c = cfg(&server);
    c.reasoning.prefix.clear();
    c.reasoning.suffix.clear();
    llm_match::generate_move(&mut m, &c, |_, _| panic!("unexpected rejection"))
        .await
        .unwrap();
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts[0], prompts[1]);
    assert!(prompts[2].contains("A plain observation, not JSON."));
    assert!(m.wrong_moves().is_empty());
}

#[tokio::test]
async fn silent_request_retries_keep_identical_input() {
    let server = Server::script(
        vec![
            Reply::Text("<thinking>unfinished".into()),
            Reply::Http(503),
            analysis(),
            Reply::Text("<thinking>done</thinking>".into()),
            Reply::Text("bad JSON".into()),
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut m = state();
    let c = cfg(&server);
    llm_match::generate_move(&mut m, &c, |_, _| panic!("silent retry emitted rejection"))
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert_eq!(p[0], p[1]);
    assert_eq!(p[1], p[2]);
    assert_eq!(p[3], p[4]);
    assert_eq!(p[4], p[5]);
    for prompt in p.iter() {
        assert!(!prompt.contains("unfinished"));
        assert!(!prompt.contains("bad JSON"));
    }
}
#[tokio::test]
async fn final_engine_rejection_restarts_proposer_without_reanalysing() {
    let server = Server::script(
        vec![
            analysis(),
            candidates(&[("e6+", "e7", "e6"), ("d5", "d7", "d5"), ("Nf6", "g8", "f6")]),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e6+"),
            candidates(&[("e6+", "e7", "e6"), ("e5", "e7", "e5"), ("e5", "e7", "e5")]),
            verify(2, true),
            select(2, "e5"),
        ],
        512,
    );
    let mut m = state();
    let c = cfg(&server);
    let mut rejected = Vec::new();
    llm_match::generate_move(&mut m, &c, |round, mv| {
        rejected.push((round, mv.to_string()))
    })
    .await
    .unwrap();
    assert_eq!(rejected, [(1, "e6+".into())]);
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
    assert!(m.wrong_moves().is_empty());
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 9);
    assert!(p[6].contains("e6+"));
    assert!(p[6].contains("ANALYST_PRIVATE_FACT"));
    assert!(!p[7].contains("e6+"));
}
#[tokio::test]
async fn wrong_coordinates_reject_without_committing_legal_san_and_retry_preserves_feedback() {
    let server = Server::script(
        vec![
            analysis(),
            candidates(&[("e5", "d7", "e5"), ("d5", "d7", "d5"), ("Nf6", "g8", "f6")]),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut c = cfg(&server);
    c.max_attempts = 1;
    let mut m = state();
    let before = m.game().position().to_fen();
    let mut rejected = Vec::new();
    assert!(
        llm_match::generate_move(&mut m, &c, |_, mv| rejected.push(mv.to_owned()))
            .await
            .is_err()
    );
    assert_eq!(m.game().position().to_fen(), before);
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert_eq!(m.wrong_moves(), ["e5"]);
    assert_eq!(rejected, ["e5"]);
    let retry = Server::script(
        vec![
            analysis(),
            normal_proposal(),
            verify(2, true),
            verify(3, false),
            select(2, "d5"),
        ],
        512,
    );
    c.endpoint = retry.endpoint.clone();
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "d7d5"]);
}
#[tokio::test]
async fn all_rejected_is_bounded_and_selector_cannot_invent_move() {
    let server = Server::script(
        vec![
            analysis(),
            normal_proposal(),
            verify(1, false),
            verify(2, false),
            verify(3, false),
            normal_proposal(),
        ],
        512,
    );
    let mut m = state();
    let c = cfg(&server);
    assert!(
        llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .is_err()
    );
    assert_eq!(server.prompts.lock().unwrap().len(), 6);
    assert!(m.wrong_moves().is_empty());
    assert_eq!(m.uci_history(), ["e2e4"]);
    let server = Server::script(
        vec![
            analysis(),
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(2, "d5"),
            select(1, "d5"),
            select(1, "e5"),
        ],
        512,
    );
    let c = cfg(&server);
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert_eq!(p[5], p[6]);
    assert_eq!(p[6], p[7]);
}
#[tokio::test]
async fn malformed_verifier_is_not_a_rejection() {
    let server = Server::script(
        vec![
            analysis(),
            normal_proposal(),
            verify(4, true),
            text(json!({"candidate_id":"c1","verdict":"MAYBE","reason":"x"})),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut m = state();
    let c = cfg(&server);
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert_eq!(p[2], p[3]);
    assert_eq!(p[3], p[4]);
}
#[tokio::test]
async fn deterministic_http_and_budget_errors_fail_immediately() {
    for (responses, tokens) in [(vec![Reply::Http(400)], 512), (vec![], 100000)] {
        let server = Server::script(responses, tokens);
        let c = cfg(&server);
        let mut m = state();
        assert!(
            llm_match::generate_move(&mut m, &c, |_, _| panic!())
                .await
                .is_err()
        );
        assert_eq!(m.uci_history(), ["e2e4"]);
        assert!(m.wrong_moves().is_empty());
        assert_eq!(
            server.prompts.lock().unwrap().len(),
            if tokens == 512 { 1 } else { 0 }
        );
    }
}
#[tokio::test]
async fn transient_status_and_timeouts_retry_without_visible_state() {
    for failure in [Reply::Http(408), Reply::Http(429), Reply::Slow("ignored")] {
        let server = Server::script(
            vec![
                failure,
                analysis(),
                normal_proposal(),
                verify(1, true),
                verify(2, false),
                verify(3, false),
                select(1, "e5"),
            ],
            512,
        );
        let mut c = cfg(&server);
        c.timeout_seconds = 1;
        let mut m = state();
        llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .unwrap();
        let p = server.prompts.lock().unwrap();
        assert_eq!(p[0], p[1]);
    }
}

#[tokio::test]
async fn fairy_variants_preserve_every_move_and_coordinate_format() {
    use llm_match::{OutputFormat, StateFormat};
    for variant in [Variant::Gothic, Variant::TerachessII] {
        for profile in [
            "classic",
            "numeric-no-history",
            "numeric-ascii",
            "hybrid",
            "uci-no-history",
            "numeric-json-no-history",
        ] {
            let mut c = llm_match::Config::load_with_profile(Some(profile)).unwrap();
            let mut m = llm_match::Match::new(variant, Color::White);
            let moves: Vec<_> = m
                .game()
                .position()
                .legal_moves()
                .into_iter()
                .take(3)
                .collect();
            let cs: Vec<_>=moves.iter().enumerate().map(|(i,mv)| {
                let mut trial=m.clone(); trial.accept_model(&mv.to_uci()).unwrap();
                let value=match c.representation.output_format {
                    OutputFormat::Semantic=>json!(trial.history()[0].content),
                    OutputFormat::Numeric=>json!(trial.numeric_history()[0].content),
                    OutputFormat::Uci=>json!(mv.to_uci()),
                    OutputFormat::NumericJson=>json!({"from":llm_match::numeric_square(mv.from),"to":llm_match::numeric_square(mv.to),"promotion":mv.promotion.map(|p|p.display_name())}),
                };
                let coord=|sq| if c.representation.state_format==StateFormat::PiecesAscii {json!(format!("{sq}"))} else {json!(llm_match::numeric_square(sq))};
                json!({"id":format!("c{}",i+1),"move":value,"from":coord(mv.from),"to":coord(mv.to),"rationale":"compact"})
            }).collect();
            let server = Server::script(
                vec![
                    analysis(),
                    text(json!({"candidates":cs})),
                    verify(1, true),
                    verify(2, false),
                    verify(3, false),
                    text(json!({"candidate_id":"c1","final_move":cs[0]["move"]})),
                ],
                512,
            );
            c.endpoint = server.endpoint.clone();
            llm_match::generate_move(&mut m, &c, |_, _| panic!())
                .await
                .unwrap();
            assert_eq!(m.uci_history(), [moves[0].to_uci()]);
            for p in server.prompts.lock().unwrap().iter() {
                assert!(!p.contains("FEN:"));
                assert!(!p.contains(&variant.starting_position().to_fen()));
                if c.representation.state_format == StateFormat::Numeric {
                    assert!(!p.contains("a b c"));
                    assert!(!p.contains("ASCII BOARD"));
                }
            }
        }
    }
}

#[tokio::test]
async fn malformed_proposer_and_connect_failures_never_change_state() {
    let invalid =
        text(json!({"candidates":[{"id":"c1","move":"e5","from":"e7","to":"e5","rationale":"x"}]}));
    let server = Server::script(vec![analysis(), invalid], 512);
    let mut c = cfg(&server);
    c.stage_max_attempts = 1;
    let mut m = state();
    assert!(
        llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .is_err()
    );
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert!(m.wrong_moves().is_empty());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    c.endpoint = format!("http://{address}");
    c.stage_max_attempts = 2;
    assert!(
        llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .is_err()
    );
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert!(m.wrong_moves().is_empty());
}

#[tokio::test]
async fn no_history_and_custom_templates_cannot_access_forbidden_stage_data() {
    let server = Server::script(
        vec![
            analysis(),
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut c = cfg(&server);
    c.classic.as_mut().unwrap().include_history = false;
    let probe = "\nPROBE history={{classic-san-history}} last={{last-move}} wrong={{wrong-move}} analysis={{analysis}} excluded={{excluded-moves}} ENDPROBE";
    c.pipeline.verifier.user_prompt = probe.into();
    c.pipeline.selector.user_prompt = probe.into();
    c.pipeline.analyst.user_prompt = "USER_SIDE {{side-to-move}} LAST {{last-move}}".into();
    let mut m = state();
    m.record_rejected_final("e6+");
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert!(p[0].contains("USER_SIDE Black LAST e4"));
    assert!(!p[0].contains("1. e4"));
    for prompt in &p[2..] {
        assert!(prompt.contains("PROBE history= last= wrong= analysis= excluded= ENDPROBE"));
        assert!(!prompt.contains("e6+"));
    }
}
