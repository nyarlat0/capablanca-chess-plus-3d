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

fn message_text(message: &Value) -> String {
    let parsed: prompt_core::ChatCompletionMessage =
        serde_json::from_value(message.clone()).unwrap();
    let mut text=parsed.text_parts().concat();
    for call in parsed.tool_calls() {
        text.push_str(&call.function.name);
        text.push_str(&call.arguments::<Value>().unwrap().to_string());
    }
    text
}
fn rendered_prompt(request: &Value) -> String {
    let mut text=String::new();
    if let Some(tools)=request["tools"].as_array() {
        for tool in tools {
            text.push_str(&tool["function"].to_string());
            text.push('\n');
        }
    }
    let messages=request["messages"].as_array().unwrap();
    for message in messages {
        text.push_str(&message_text(message));
        text.push('\n');
    }
    text
}
enum Reply {
    Text(String),
    /// Complete response assistant message, including structured tool calls.
    Message(Value),
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
                        if request.get("messages").is_none() {
                            let response = json!({"value": 42}).to_string();
                            let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response);
                            continue;
                        }
                        let prompt = rendered_prompt(&request);
                        json!({"value":token_count, "prompt":prompt})
                    }
                    "/v1/chat/completions" => {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        assert!(request.get("prompt").is_none());
                        let prompt = rendered_prompt(&request);
                        recorded.lock().unwrap().push(prompt);
                        recorded_requests.lock().unwrap().push(request);
                        match answers.next().expect("unexpected extra generation") {
                            Reply::Text(text) => {
                                json!({"choices":[{"message":{"role":"assistant","content":text}}]})
                            }
                            Reply::Message(message) => {
                                json!({"choices":[{"message":message,"finish_reason":"tool_calls"}]})
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
fn grounding() -> Reply {
    grounding_for(state().game().position())
}

#[tokio::test]
async fn one_grounder_full_collects_sequentially_retries_current_piece_and_builds_graph() {
    use llm_match::pipeline::{
        Stage,
        debug::{self, Inputs},
    };
    let server = Server::script(
        vec![
            Reply::Text("The king controls its three adjacent in-board squares.\n```json\n{\"controls\":[\"h7\",\"g8\",\"g7\"],\"xray\":[]}\n```".into()),
            text(json!({"controls":["d1","d2","f1","f2"],"xray":[]})), // valid schema, missing e2: retry only this piece
            text(json!({"controls":["d1","d2","e2","f1","f2"],"xray":[]})),
            text(json!({"controls":["b3"],"xray":[]})),
            text(json!({"controls":["a2","b1","c1","d1","e1"],"xray":[{"direction":"E","first":"e1","second":null},{"direction":"N","first":"a2","second":null}]})),
        ],
        512,
    );
    let mut config = cfg(&server);
    config.pipeline.one_grounder.content = "ISOLATED_ONE_PIECE".into();
    config.pipeline.one_grounder.user_prompt = "selected={{selected-piece}}".into();
    let m = llm_match::Match::from_fen(
        Variant::Classic,
        Color::White,
        "7k/8/8/8/8/8/P7/R3K3 w - - 0 1",
    )
    .unwrap();
    let before = m.game().position().to_fen();
    let output = debug::run(&m, &config, Stage::OneGrounderFull, &Inputs::default())
        .await
        .unwrap();
    let graph = llm_match::grounder::ControlGraph::parse(m.game().position(), &output).unwrap();
    assert_eq!(
        graph
            .map()
            .pieces
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["bK_h8", "wK_e1", "wP_a2", "wR_a1"]
    );
    assert_eq!(
        graph.get_piece("wR_a1").unwrap().defends_allies,
        ["wK_e1", "wP_a2"]
    );
    assert_eq!(graph.get_piece("wP_a2").unwrap().defended_by, ["wR_a1"]);
    assert_eq!(graph.get_attackers("d1").unwrap(), ["wK_e1", "wR_a1"]);
    assert_eq!(m.game().position().to_fen(), before);
    assert!(m.uci_history().is_empty() && m.wrong_moves().is_empty());
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 5);
    for (prompt, selected) in prompts.iter().zip([
        "Black King: h8",
        "White King: e1",
        "White King: e1",
        "White Pawn: a2",
        "White Rook: a1",
    ]) {
        assert!(prompt.contains(&format!("selected={selected}")));
        assert!(!prompt.contains("controls")); // previous results never enter context
    }
    assert_eq!(prompts[1], prompts[2]);
    let requests = server.requests.lock().unwrap();
    let images: Vec<_> = requests
        .iter()
        .map(|r| {
            r["messages"].as_array().unwrap().last().unwrap()["content"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["image_url"]["url"]
                .clone()
        })
        .collect();
    assert!(images.windows(2).all(|w| w[0] == w[1]));
}

#[tokio::test]
async fn one_grounder_full_stops_on_failure_and_rejects_extra_inputs() {
    use llm_match::pipeline::{
        Stage,
        debug::{self, Inputs},
    };
    let server = Server::script(
        vec![
            text(json!({"controls":[],"xray":[]})),
            text(json!({"controls":["z9"],"xray":[]})),
        ],
        512,
    );
    let mut config = cfg(&server);
    config.stage_max_attempts = 1;
    let m = state();
    let before = m.game().position().to_fen();
    let error = debug::run(&m, &config, Stage::OneGrounderFull, &Inputs::default())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("one-grounder-full failed for"));
    assert_eq!(server.prompts.lock().unwrap().len(), 2);
    assert_eq!(m.game().position().to_fen(), before);
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert!(m.wrong_moves().is_empty());
    let extra = Inputs {
        selected_piece: Some("b1".into()),
        ..Default::default()
    };
    assert!(
        debug::run(&m, &config, Stage::OneGrounderFull, &extra)
            .await
            .is_err()
    );
    let fairy = llm_match::Match::new(Variant::Gothic, Color::White);
    assert!(
        debug::run(&fairy, &config, Stage::OneGrounderFull, &Inputs::default())
            .await
            .is_err()
    );
    assert_eq!(server.prompts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn one_grounder_is_isolated_and_retries_without_feedback() {
    use llm_match::pipeline::{
        Stage,
        debug::{self, Inputs},
    };
    let server = Server::script(
        vec![
            Reply::Text("<thinking>unfinished".into()),
            text(json!({"version":1,"controls":[]})),
            Reply::Text(
                "<thinking>private</thinking>The knight controls these squares, including its own pawn on e7.\n```json\n{\"controls\":[\"h6\",\"f6\",\"e7\"],\"xray\":[]}\n```"
                    .into(),
            ),
        ],
        512,
    );
    let mut config = cfg(&server);
    config.stage_presets.one_grounder =
        Some(prompt_core::Preset::from_value(json!({"genamt":321})).unwrap());
    config.pipeline.one_grounder.content = "ONE_ONLY {{selected-piece}}".into();
    let m = state();
    let before = m.game().position().to_fen();
    let inputs = Inputs {
        selected_piece: Some("g8".into()),
        ..Default::default()
    };
    let output = debug::run(&m, &config, Stage::OneGrounder, &inputs)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap(),
        json!({"controls":["e7","f6","h6"],"xray":[]})
    );
    assert_eq!(m.game().position().to_fen(), before);
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert!(m.wrong_moves().is_empty());
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 3);
    assert!(prompts[0].contains("ONE_ONLY Black Knight: g8"));
    assert!(!prompts[0].contains("ACCEPTED MOVE HISTORY"));
    assert_eq!(prompts[0], prompts[1]);
    assert_eq!(prompts[1], prompts[2]);
    for request in server.requests.lock().unwrap().iter() {
        assert_eq!(request["max_tokens"], 321);
    }
}

#[tokio::test]
async fn one_grounder_invalid_selection_never_calls_model() {
    use llm_match::pipeline::{
        Stage,
        debug::{self, Inputs},
    };
    let server = Server::script(vec![], 512);
    let config = cfg(&server);
    let m = state();
    assert!(
        debug::run(&m, &config, Stage::OneGrounder, &Inputs::default())
            .await
            .is_err()
    );
    for selected in ["a3", "bN_b1", "A1"] {
        let inputs = Inputs {
            selected_piece: Some(selected.into()),
            ..Default::default()
        };
        assert!(
            debug::run(&m, &config, Stage::OneGrounder, &inputs)
                .await
                .is_err()
        );
    }
    let inputs = Inputs {
        selected_piece: Some("b1".into()),
        ..Default::default()
    };
    assert!(
        debug::run(&m, &config, Stage::Grounder, &inputs)
            .await
            .is_err()
    );
    let fairy = llm_match::Match::new(Variant::Gothic, Color::White);
    assert!(
        debug::run(&fairy, &config, Stage::OneGrounder, &inputs)
            .await
            .is_err()
    );
    assert!(server.prompts.lock().unwrap().is_empty());
}
fn grounding_for(position: &capablanca_chess_plus::Position) -> Reply {
    if position.rules().name() != Variant::Classic.rules().name() {
        return Reply::Text("GROUNDED_FACT".into());
    }
    // Keep the mock's accepted response geometrically correct. Production
    // Grounder still obtains the candidate map from the MODEL, not this helper.
    let map=llm_match::grounder::engine_map(position).unwrap();
    Reply::Text(format!(
        "<thinking>GROUNDER_PRIVATE_REASONING</thinking>{}",
        serde_json::to_string(&map).unwrap()
    ))
}

#[test]
fn grounder_cli_reports_bad_maps_and_retries_without_model_feedback() {
    let valid = match grounding() {
        Reply::Text(text) => text.split("</thinking>").nth(1).unwrap().to_owned(),
        _ => unreachable!(),
    };
    let invalid = r#"{"version":1,"pieces":[],"unexpected_marker":"BAD_MAP_DIAGNOSTIC"}"#;
    let server = Server::script(
        vec![
            Reply::Text("<think>SILENT_TRUNCATION".into()),
            Reply::Text(invalid.into()),
            Reply::Text(valid.clone()),
        ],
        512,
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_llm-match"))
        .env(
            "LLM_MATCH_ENV",
            concat!(env!("CARGO_MANIFEST_DIR"), "/.env"),
        )
        .env("LLM_STAGE_MAX_ATTEMPTS", "3")
        .args([
            "--stage",
            "grounder",
            "--moves-format",
            "san",
            "--moves",
            "e4",
            "--url",
            &server.endpoint,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Grounder attempt 2 rejected"));
    assert!(stderr.contains("unknown field"));
    assert!(stderr.contains(invalid));
    assert!(!stderr.contains("SILENT_TRUNCATION"));
    let result = String::from_utf8(output.stdout).unwrap();
    let graph =
        llm_match::grounder::ControlGraph::parse(state().game().position(), &result).unwrap();
    assert_eq!(graph.map().pieces.len(), 32);
    let prompts = server.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 3);
    assert_eq!(prompts[0], prompts[1]);
    assert_eq!(prompts[1], prompts[2]);
    assert!(!prompts[2].contains("BAD_MAP_DIAGNOSTIC"));
}

#[tokio::test]
async fn rejected_grounder_maps_never_change_match_state() {
    let server = Server::script(vec![text(json!({"version":1,"pieces":[]}))], 512);
    let mut config = cfg(&server);
    config.stage_max_attempts = 1;
    let m = state();
    let before = m.game().position().to_fen();
    let error = llm_match::pipeline::debug::run(
        &m,
        &config,
        llm_match::pipeline::Stage::Grounder,
        &Default::default(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("missing pieces"));
    assert_eq!(m.game().position().to_fen(), before);
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert!(m.wrong_moves().is_empty());
    assert_eq!(server.prompts.lock().unwrap().len(), 1);
}
fn candidates(moves: &[(&str, &str, &str)]) -> Reply {
    text(
        json!({"candidates": moves.iter().enumerate().map(|(i,(mv,from,to))|
        json!({"id":format!("c{}",i+1),"move":mv,"from":from,"to":to,"rationale":"PROPOSER_PRIVATE_RATIONALE"})).collect::<Vec<_>>()}),
    )
}
fn verify(id: usize, accept: bool) -> Reply {
    text(
        json!({"verdict":if accept {"ACCEPT"} else {"REJECT"},"tactical_result":format!("TACTICAL_RESULT_{id}")}),
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
    m.prepare_grounder_test(true);
    m
}

#[tokio::test]
async fn debug_runs_only_requested_stage_and_never_commits() {
    use llm_match::pipeline::{
        Stage,
        debug::{self, Inputs},
    };
    let candidate = json!({"id":"c1","move":"e5","from":"e7","to":"e5","rationale":"SECRET_IDEA"});
    for (stage, reply, inputs) in [
        (Stage::Grounder, grounding(), Inputs::default()),
        (
            Stage::Proposer,
            normal_proposal(),
            Inputs {
                analysis: Some("SUPPLIED_FACTS".into()),
                ..Default::default()
            },
        ),
        (
            Stage::Verifier,
            verify(1, true),
            Inputs {
                analysis: Some("SUPPLIED_FACTS".into()),
                candidate: Some(candidate.clone()),
                ..Default::default()
            },
        ),
        (
            Stage::Selector,
            select(1, "e5"),
            Inputs {
                accepted_candidates: Some(json!([
                    {"id":"c1","move":"e5","from":"e7","to":"e5","tactical_result":"SUPPLIED_RESULT"}
                ])),
                ..Default::default()
            },
        ),
    ] {
        let server = Server::script(vec![reply], 512);
        let c = cfg(&server);
        let m = state();
        let before = m.game().position().to_fen();
        let result = debug::run(&m, &c, stage, &inputs).await.unwrap();
        assert!(!result.is_empty());
        assert_eq!(m.game().position().to_fen(), before);
        assert_eq!(m.uci_history(), ["e2e4"]);
        assert!(m.wrong_moves().is_empty());
        let p = server.prompts.lock().unwrap();
        assert_eq!(p.len(), 1);
        assert!(!p[0].contains("SECRET_IDEA"));
        assert_eq!(
            p[0].contains("SUPPLIED_FACTS"),
            matches!(stage, Stage::Proposer | Stage::Verifier)
        );
        assert_eq!(p[0].contains("SUPPLIED_RESULT"), stage == Stage::Selector);
    }
}

#[tokio::test]
async fn debug_missing_inputs_and_illegal_candidates_fail_before_network() {
    use llm_match::pipeline::{
        Stage,
        debug::{self, Inputs},
    };
    let server = Server::script(vec![], 512);
    let c = cfg(&server);
    let m = state();
    for stage in [Stage::Proposer, Stage::Verifier, Stage::Selector] {
        assert!(debug::run(&m, &c, stage, &Inputs::default()).await.is_err());
    }
    let inputs = Inputs {
        analysis: Some("facts".into()),
        candidate: Some(json!({
            "id":"c1","move":"e5","from":"d7","to":"e5"
        })),
        ..Default::default()
    };
    assert!(debug::run(&m, &c, Stage::Verifier, &inputs).await.is_err());
    assert!(debug::run(&m, &c, Stage::Grounder, &inputs).await.is_err());
    assert!(server.prompts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn debug_retries_only_the_chosen_stage_and_strips_reasoning() {
    use llm_match::pipeline::{
        Stage,
        debug::{self, Inputs},
    };
    let server = Server::script(
        vec![
            Reply::Text("<thinking>unfinished".into()),
            Reply::Http(503),
            grounding(),
        ],
        512,
    );
    let c = cfg(&server);
    let m = state();
    assert_eq!(
        debug::run(&m, &c, Stage::Grounder, &Inputs::default())
            .await
            .unwrap(),
        llm_match::grounder::ControlGraph::parse(
            m.game().position(),
            &match grounding() {
                Reply::Text(t) => t.split("</thinking>").nth(1).unwrap().to_owned(),
                _ => unreachable!(),
            }
        )
        .unwrap()
        .normalized_json()
        .unwrap()
    );
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 3);
    assert_eq!(p[0], p[1]);
    assert_eq!(p[1], p[2]);
    assert!(m.wrong_moves().is_empty());
}

#[tokio::test]
async fn four_stage_skips_analyst_and_shares_grounded_facts() {
    let server = Server::script(
        vec![
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut c = cfg(&server);
    c.pipeline.grounder.content = "GROUNDER_ONLY {{#if analysis}}UNEXPECTED_ANALYSIS{{/if}}".into();
    c.pipeline.proposer.content = "PROPOSER_ONLY".into();
    c.pipeline.verifier.content = "VERIFIER_ONLY".into();
    c.pipeline.selector.content = "SELECTOR_ONLY".into();
    let mut m = state();
    let before_fen = m.game().position().to_fen();
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
    let requests = server.requests.lock().unwrap();
    let mut previous_image = None;
    for request in requests.iter() {
        let messages = request["messages"].as_array().unwrap();
        let last_user = messages.iter().rposition(|m| m["role"] == "user").unwrap();
        for (i, message) in messages.iter().enumerate() {
            let message: prompt_core::ChatCompletionMessage =
                serde_json::from_value(message.clone()).unwrap();
            assert_eq!(message.image_count(), usize::from(i == last_user));
        }
        let content = messages[last_user]["content"].as_array().unwrap();
        assert!(message_text(&messages[last_user]).contains(&before_fen));
        let image = content.last().unwrap()["image_url"]["url"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(image.starts_with("data:image/png;base64,"));
        if let Some(previous) = &previous_image {
            assert_eq!(&image, previous);
        }
        previous_image = Some(image);
    }
    assert!(
        m.history()
            .iter()
            .all(|m| !m.content.contains("data:image"))
    );
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 5);
    for (index, marker) in [
        (0, "PROPOSER_ONLY"),
        (1, "VERIFIER_ONLY"),
        (2, "VERIFIER_ONLY"),
        (3, "VERIFIER_ONLY"),
        (4, "SELECTOR_ONLY"),
    ] {
        assert!(p[index].contains(marker));
    }
    assert!(
        p.iter()
            .all(|prompt| !prompt.contains("ANALYST_MUST_NOT_RUN")
                && !prompt.contains("UNEXPECTED_ANALYSIS"))
    );
    for prompt in &p[0..4] {
        assert!(prompt.contains("\"version\": 1"));
    }
    assert!(!p[4].contains("\"version\": 1"));
    assert!(
        p.iter()
            .all(|prompt| !prompt.contains("GROUNDER_PRIVATE_REASONING"))
    );
    assert!(p[4].contains("TACTICAL_RESULT_1"));
    assert!(!p[4].contains("TACTICAL_RESULT_2"));
    assert!(!p[4].contains("TACTICAL_RESULT_3"));
    assert!(p[4].contains("tactical_result"));
    assert!(!p[1].contains("rationale"));
    assert!(!p[4].contains("rationale"));
}

#[tokio::test]
async fn four_stage_reuses_grounder_after_illegal_candidates_and_deduplicates() {
    let server = Server::script(
        vec![
            candidates(&[
                ("e6+", "e7", "e6"),
                ("Qh4", "d8", "h4"),
                ("Ke7", "e8", "e7"),
            ]),
            candidates(&[("e5", "e7", "e5"), ("e5", "e7", "e5"), ("Nf6", "g8", "f6")]),
            verify(1, true),
            verify(3, true),
            select(1, "e5"),
        ],
        512,
    );
    let c = cfg(&server);
    let mut m = state();
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 5);
    for prompt in &p[0..4] {
        assert!(prompt.contains("\"version\": 1"));
    }
    assert!(p[1].contains("e6+"));
    assert!(p[4].contains("TACTICAL_RESULT_1") && p[4].contains("TACTICAL_RESULT_3"));
    assert!(!p[4].contains("TACTICAL_RESULT_2"));
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
}

#[tokio::test]
async fn four_stage_handles_every_variant_without_engine_hints() {
    for variant in Variant::ALL {
        let mut m = llm_match::Match::new(variant, Color::White);
        // Test fixtures only: the runtime never sends this engine move list.
        let moves: Vec<_> = m
            .game()
            .position()
            .legal_moves()
            .into_iter()
            .take(3)
            .collect();
        let cs: Vec<_> = moves
            .iter()
            .enumerate()
            .map(|(i, mv)| {
                let text = if variant == Variant::Classic {
                    m.game().position().san(*mv).unwrap()
                } else {
                    mv.to_uci()
                };
                json!({"id":format!("c{}",i+1),"move":text,"from":mv.from.to_string(),
                "to":mv.to.to_string(),"rationale":"FIXTURE_PRIVATE_IDEA"})
            })
            .collect();
        let mut replies = vec![
                text(json!({"candidates":cs})),
                verify(1, true),
                verify(2, false),
                verify(3, false),
                text(json!({"candidate_id":"c1","final_move":cs[0]["move"]})),
            ];
        if variant != Variant::Classic { replies.insert(0, grounding_for(m.game().position())); }
        let server = Server::script(replies, 512);
        let mut c = if variant == Variant::Classic {
            cfg(&server)
        } else {
            llm_match::Config::load_with_profile(Some("uci-no-history")).unwrap()
        };
        c.endpoint = server.endpoint.clone();
        c.reasoning = cfg(&server).reasoning;
        let mv = llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .unwrap();
        assert_eq!(mv, moves[0]);
        let p = server.prompts.lock().unwrap();
        assert_eq!(p.len(), if variant == Variant::Classic {5} else {6});
        assert!(!p[0].contains("FIXTURE_PRIVATE_IDEA"));
        assert!(p[1..].iter().all(|s| !s.contains("FIXTURE_PRIVATE_IDEA")));
    }
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
    assert_eq!(server.prompts.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn each_stage_sends_its_own_sampler_and_response_budget() {
    let server = Server::script(
        vec![
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut c = cfg(&server);
    let preset = |tokens, temp| {
        Some(prompt_core::Preset::from_value(json!({"genamt":tokens,"temp":temp})).unwrap())
    };
    c.stage_presets = llm_match::StagePresets {
        one_grounder: None,
        grounder: preset(151, 0.15),
        proposer: preset(202, 0.2),
        verifier: preset(303, 0.3),
        selector: preset(404, 0.4),
    };
    let mut m = state();
    llm_match::generate_move(&mut m, &c, |_, _| panic!("unexpected rejection"))
        .await
        .unwrap();
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    for (request, (tokens, temperature)) in requests.iter().zip([
        (202, 0.2),
        (303, 0.3),
        (303, 0.3),
        (303, 0.3),
        (404, 0.4),
    ]) {
        assert_eq!(request["max_tokens"], tokens);
        assert_eq!(request["temperature"], temperature);
    }
}

#[tokio::test]
async fn engine_gate_filters_illegal_and_mismatched_candidates_without_committing() {
    let server = Server::script(
        vec![
            candidates(&[("e6+", "e7", "e6"), ("e5", "d7", "e5"), ("Qh4", "d8", "h4")]),
            candidates(&[("e6+", "e7", "e6"), ("e5", "e7", "e5"), ("d5", "d7", "d5")]),
            verify(2, false),
            verify(3, true),
            select(3, "d5"),
        ],
        512,
    );
    let mut m = state();
    let c = cfg(&server);
    llm_match::generate_move(&mut m, &c, |_, _| {
        panic!("gate must not emit final rejection")
    })
    .await
    .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "d7d5"]);
    assert!(m.wrong_moves().is_empty());
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 5);
    assert!(p[1].contains("e6+"));
    assert!(p[1].contains("Qh4"));
    assert!(p[1].contains("\"version\": 1"));
    assert!(!p[1].contains("ANALYST_PRIVATE_FACT"));
    assert!(!p[2].contains("e6+"));
    assert!(p[2].contains(r#""move":"e5""#));
    assert!(p[3].contains(r#""move":"d5""#));
}

#[tokio::test]
async fn incorrect_metadata_cannot_poison_same_legal_move_and_valid_duplicates_are_deduplicated() {
    let server = Server::script(
        vec![
            candidates(&[
                ("Nxd5", "c6", "d5"),
                ("Nxd5", "f6", "d5"),
                ("Nxd5", "f6", "d5"),
            ]),
            verify(2, true),
            select(2, "Nxd5"),
        ],
        512,
    );
    let mut m = llm_match::Match::from_fen(
        Variant::Classic,
        Color::Black,
        "r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5",
    )
    .unwrap();
    llm_match::generate_move(&mut m, &cfg(&server), |_, _| panic!("unexpected rejection"))
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["f6d5"]);
    assert!(m.wrong_moves().is_empty());
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 3); // only one verifier call, for the first valid candidate
    assert!(p[1].contains(r#""from":"f6""#));
    assert!(!p[1].contains(r#""from":"c6""#));
    assert!(!p[1].contains("candidate_id"));
    // Grounder now includes piece ids; candidate identity is still excluded.
    assert!(!p[1].contains(r#""id":"c2""#));
    assert!(p[2].contains(r#""id":"c2""#));
    assert!(p[2].contains(r#""tactical_result":"TACTICAL_RESULT_2""#));
    assert!(!p[2].contains(r#""id":"c1""#));
    assert!(!p[2].contains(r#""id":"c3""#));
}

#[tokio::test]
async fn exhausted_gate_does_not_mutate_position_history_or_rejected_finals() {
    let server = Server::script(
        vec![
            candidates(&[("e6+", "e7", "e6"), ("e5", "d7", "e5"), ("Qh4", "d8", "h4")]),
        ],
        512,
    );
    let mut c = cfg(&server);
    c.max_attempts = 1;
    let mut m = state();
    let before = m.game().position().to_fen();
    assert!(
        llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .is_err()
    );
    assert_eq!(m.game().position().to_fen(), before);
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert!(m.wrong_moves().is_empty());
    assert_eq!(server.prompts.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn malformed_verifier_is_not_a_rejection() {
    let server = Server::script(
        vec![
            normal_proposal(),
            text(json!({"candidate_id":"c1","verdict":"ACCEPT","tactical_result":"x"})),
            text(json!({"verdict":"ACCEPT","tactical_result":"  "})),
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
    assert_eq!(p[1], p[2]);
    assert_eq!(p[2], p[3]);
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
        assert_eq!(p[0], p[0]);
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
                    grounding_for(m.game().position()),
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
                // Representation-specific textual rules are unchanged; every
                // stage now additionally receives an image and matching FEN.
                assert!(p.contains(&variant.starting_position().to_fen()));
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
    let server = Server::script(vec![invalid], 512);
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
async fn one_grounder_retries_engine_incorrect_controls_and_keeps_sorted_output() {
    use llm_match::pipeline::{Stage,debug::{self,Inputs}};
    let server=Server::script(vec![
        text(json!({"controls":["f6","e7"],"xray":[]})),
        text(json!({"controls":["h6","e7","f6"],"xray":[]})),
    ],512);
    let config=cfg(&server);
    let m=state();
    let before=m.game().position().to_fen();
    let result=debug::run(&m,&config,Stage::OneGrounder,&Inputs {
        selected_piece:Some("g8".into()), ..Default::default()
    }).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&result).unwrap(),
        json!({"controls":["e7","f6","h6"],"xray":[]}));
    assert_eq!(m.game().position().to_fen(),before);
    let prompts=server.prompts.lock().unwrap();
    assert_eq!(prompts.len(),2);
    assert_eq!(prompts[0],prompts[1]); // error isn't fed to the model
}

#[tokio::test]
async fn full_grounder_retries_model_geometry_and_normalizes_input_order() {
    use llm_match::pipeline::{Stage,debug::{self,Inputs}};
    let m=state();
    let oracle=llm_match::grounder::engine_map(m.game().position()).unwrap();
    let mut bad=oracle.clone();
    bad.pieces.iter_mut().find(|p|p.id=="bN_g8").unwrap()
        .controls.retain(|s|s!="h6");
    let mut unordered=oracle.clone();
    unordered.pieces.reverse();
    for piece in &mut unordered.pieces {
        piece.controls.reverse();
        piece.xray.reverse();
    }
    let server=Server::script(vec![
        text(serde_json::to_value(bad).unwrap()),
        text(serde_json::to_value(unordered).unwrap()),
    ],512);
    let config=cfg(&server);
    let result=debug::run(&m,&config,Stage::Grounder,&Inputs::default()).await.unwrap();
    let validated: llm_match::grounder::ControlMap=serde_json::from_str(&result).unwrap();
    assert_eq!(validated,oracle);
    let prompts=server.prompts.lock().unwrap();
    assert_eq!(prompts.len(),2);
    assert_eq!(prompts[0],prompts[1]);
}

#[tokio::test]
async fn one_grounder_retries_when_xray_omits_a_blocked_ray() {
    use llm_match::pipeline::{Stage,debug::{self,Inputs}};
    let fen="r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5";
    let m=llm_match::Match::from_fen(Variant::Classic,Color::Black,fen).unwrap();
    let oracle=llm_match::grounder::engine_map(m.game().position()).unwrap();
    let queen=oracle.pieces.iter().find(|p|p.id=="bQ_d8").unwrap();
    let mut incomplete=queen.clone();
    incomplete.xray.retain(|r|r.direction!="S");
    let mut valid=queen.clone();
    valid.controls.reverse();
    valid.xray.reverse();

    let server=Server::script(vec![
        text(json!({"controls":incomplete.controls,"xray":incomplete.xray})),
        text(json!({"controls":valid.controls,"xray":valid.xray})),
    ],512);
    let config=cfg(&server);
    let result=debug::run(&m,&config,Stage::OneGrounder,&Inputs {
        selected_piece:Some("d8".into()),..Default::default()
    }).await.unwrap();
    let parsed: llm_match::grounder::OnePieceControls=serde_json::from_str(&result).unwrap();
    assert_eq!(parsed.controls,queen.controls);
    assert_eq!(parsed.xray,queen.xray);
    let prompts=server.prompts.lock().unwrap();
    assert_eq!(prompts.len(),2);
    assert_eq!(prompts[0],prompts[1]);
}

#[tokio::test]
async fn one_grounder_can_inspect_occupants_and_continue_in_same_attempt() {
    use llm_match::pipeline::{Stage,debug::{self,Inputs}};

    // The model chooses these squares, not the Rust geometry checker.
    let call = |id: &str, square: &str| json!({
        "id":id,"type":"function",
        "function":{"name":"piece_at","arguments":json!({"square":square}).to_string()}
    });
    let server=Server::script(vec![
        Reply::Message(json!({
            "role":"assistant","content":null,
            "tool_calls":[call("call-c3","c3"),call("call-c4","c4")]
        })),
        text(json!({
            "controls":["c3","d3","e4","d2","d1","c1","b1","b2","b3","a4"],
            "xray":[
                {"direction":"N","first":"c3","second":"c5"},
                {"direction":"NE","first":"e4","second":"g6"},
                {"direction":"E","first":"d2","second":"e2"},
                {"direction":"W","first":"b2","second":"a2"}
            ]
        })),
    ],512);
    let config=cfg(&server);
    let fen="r3k2r/p1ppqpb1/bn2pnp1/2pP4/1p2P3/2N2N2/PPQBBPPP/R3K2R w KQkq - 0 1";
    let m=llm_match::Match::from_fen(Variant::Classic,Color::White,fen).unwrap();
    let before=m.game().position().to_fen();
    let output=debug::run(&m,&config,Stage::OneGrounder,&Inputs{
        selected_piece:Some("c2".into()),..Default::default()
    }).await.unwrap();
    let piece: llm_match::grounder::OnePieceControls=serde_json::from_str(&output).unwrap();
    assert_eq!(piece.controls.len(),10);
    assert_eq!(m.game().position().to_fen(),before);

    let requests=server.requests.lock().unwrap();
    assert_eq!(requests.len(),2);
    for request in requests.iter() {
        assert_eq!(request["tools"][0]["function"]["name"],"piece_at");
        assert_eq!(request["tool_choice"],"auto");
    }
    let continuation=&requests[1]["messages"];
    assert!(continuation.as_array().unwrap().len()>requests[0]["messages"].as_array().unwrap().len());
    let tool_messages:Vec<_>=continuation.as_array().unwrap().iter()
        .filter(|msg|msg["role"]=="tool").collect();
    assert_eq!(tool_messages.len(),2);
    assert_eq!(tool_messages[0]["tool_call_id"],"call-c3");
    assert_eq!(tool_messages[1]["tool_call_id"],"call-c4");
    let first:Value=serde_json::from_str(tool_messages[0]["content"].as_str().unwrap()).unwrap();
    let second:Value=serde_json::from_str(tool_messages[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(first,json!({"square":"c3","piece":"wN_c3"}));
    assert_eq!(second,json!({"square":"c4","piece":null}));
    assert_eq!(server.prompts.lock().unwrap().len(),2); // one attempt, two generations
}

#[tokio::test]
async fn tool_rounds_have_no_application_call_limit() {
    use llm_match::pipeline::{Stage,debug::{self,Inputs}};
    let calls:Vec<_>=(0..40).map(|i| json!({
        "id":format!("lookup-{i}"),"type":"function",
        "function":{"name":"piece_at","arguments":json!({"square":"a1"}).to_string()}
    })).collect();
    let server=Server::script(vec![
        Reply::Message(json!({"role":"assistant","content":null,"tool_calls":calls})),
        text(json!({"controls":["d1","d2","e2","f1","f2"],"xray":[]})),
    ],512);
    let config=cfg(&server);
    let m=llm_match::Match::from_fen(Variant::Classic,Color::White,
        "7k/8/8/8/8/8/P7/R3K3 w - - 0 1").unwrap();
    debug::run(&m,&config,Stage::OneGrounder,&Inputs{
        selected_piece:Some("e1".into()),..Default::default()
    }).await.unwrap();
    let requests=server.requests.lock().unwrap();
    assert_eq!(requests.len(),2);
    let tool_count=requests[1]["messages"].as_array().unwrap().iter()
        .filter(|m|m["role"]=="tool").count();
    assert_eq!(tool_count,40);
}

#[tokio::test]
async fn piece_at_returns_an_error_for_invalid_square_without_stopping_reasoning() {
    use llm_match::pipeline::{Stage,debug::{self,Inputs}};
    let server=Server::script(vec![
        Reply::Message(json!({"role":"assistant","content":null,
            "tool_calls":[{"id":"bad-square","type":"function","function":{
                "name":"piece_at","arguments":json!({"square":"c9"}).to_string()
            }}]})),
        text(json!({"controls":["g7","g8","h7"],"xray":[]})),
    ],512);
    let config=cfg(&server);
    let m=llm_match::Match::from_fen(Variant::Classic,Color::White,
        "7k/8/8/8/8/8/P7/R3K3 w - - 0 1").unwrap();
    debug::run(&m,&config,Stage::OneGrounder,&Inputs{
        selected_piece:Some("h8".into()),..Default::default()
    }).await.unwrap();
    let requests=server.requests.lock().unwrap();
    let call_result=&requests[1]["messages"].as_array().unwrap().last().unwrap()["content"];
    let payload:Value=serde_json::from_str(call_result.as_str().unwrap()).unwrap();
    assert!(payload["error"].as_str().unwrap().contains("square must be"));
}
