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
    if let prompt_core::ChatCompletionMessage::Assistant {
        reasoning_content: Some(reasoning), ..
    } = &parsed {
        text.push_str("<think>");
        text.push_str(reasoning);
        text.push_str("</think>");
    }
    for call in parsed.tool_calls() {
        // Emulate Qwen's Jinja template: function arguments are NOT JSON.
        text.push_str(&format!("<tool_call><function={}>",call.function.name));
        let arguments=call.arguments::<Value>().unwrap();
        for (name,value) in arguments.as_object().unwrap() {
            let argument=value.as_str().map(str::to_owned).unwrap_or_else(||value.to_string());
            text.push_str(&format!("<parameter={name}>\n{argument}\n</parameter>"));
        }
        text.push_str("</function></tool_call>");
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
            stage == Stage::Verifier
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
    // Classic Proposer no longer needs an analysis file: the engine supplies
    // a complete readable control report before the model request.
    let preview = debug::preview(&m, &c, Stage::Proposer, &Inputs::default()).unwrap();
    assert!(preview.contains("VERIFIED GROUNDER GEOMETRY OF THE CURRENT POSITION"));
    for stage in [Stage::Verifier, Stage::Selector] {
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


fn classic_lines() -> Reply {
    text(json!({"candidates":[
        {"id":"c1","move":"e5","counterplay":["Nf3"]},
        {"id":"c2","move":"d5","counterplay":["exd5"]}
    ]}))
}

#[tokio::test]
async fn classic_pipeline_uses_proposer_grounder_deltas_and_selector_without_image() {
    let server=Server::script(vec![
        classic_lines(),
        text(json!({"candidate_id":"c1"}))
    ],512);
    let mut m=state();
    m.set_auto_grounder_updates(true);
    let mut config=cfg(&server);
    config.send_board_image=false;
    let original=m.game().position().to_fen();
    let mv=llm_match::generate_move(&mut m,&config,|_,_|panic!("unexpected rejection"))
        .await.unwrap();
    assert_eq!(mv.to_uci(),"e7e5");
    assert_ne!(original,m.game().position().to_fen());
    assert_eq!(m.uci_history(),["e2e4","e7e5"]);
    let requests=server.requests.lock().unwrap();
    assert_eq!(requests.len(),2);
    assert!(requests.iter().all(|req|req["messages"].as_array().unwrap().iter().all(|msg|{
        let parsed:prompt_core::ChatCompletionMessage=serde_json::from_value(msg.clone()).unwrap();
        parsed.image_count()==0
    })));
    assert_eq!(requests[0]["chat_template_kwargs"]["enable_thinking"],false);
    assert_eq!(requests[1]["chat_template_kwargs"]["enable_thinking"],false);
    assert!(requests.iter().all(|req|req.get("tools").is_none()));
    let selector=rendered_prompt(&requests[1]);
    assert!(selector.contains("free_targets_after"));
    assert!(selector.contains("after_move"));
    assert!(selector.contains("counterplay"));
    assert!(selector.contains("square_control_deltas"));
    assert!(!selector.contains("wP_e4"));
}

#[tokio::test]
async fn illegal_reply_returns_actionable_feedback_to_proposer_before_selector() {
    let server=Server::script(vec![
        text(json!({"candidates":[
            {"id":"c1","move":"e5","counterplay":["Bh6"]},
            {"id":"c2","move":"d5","counterplay":["exd5"]}
        ]})),
        text(json!({"candidate_id":"c2"}))
    ],512);
    let mut m=state();
    m.set_auto_grounder_updates(true);
    let config=cfg(&server);
    let mv=llm_match::generate_move(&mut m,&config,|_,_|{}).await.unwrap();
    assert_eq!(mv.to_uci(),"d7d5");
    let requests=server.requests.lock().unwrap();
    assert_eq!(requests.len(),2,"bad reply must not force Proposer retry");
    let selector=rendered_prompt(&requests[1]);
    assert!(selector.contains("counterplay"));
    assert!(selector.contains("d5"));
    assert!(!selector.contains("Bh6"),"illegal reply must be filtered");
    assert_eq!(m.uci_history(),["e2e4","d7d5"]);
}

#[tokio::test]
async fn engine_grounder_geometry_is_available_without_flag_or_llm_calls() {
    let server=Server::script(vec![
        classic_lines(),
        text(json!({"candidate_id":"c2"}))
    ],512);
    let mut m=state();
    let config=cfg(&server);
    assert!(!m.uci_history().is_empty());
    let mv=llm_match::generate_move(&mut m,&config,|_,_|{}).await.unwrap();
    assert_eq!(mv.to_uci(),"d7d5");
    let requests=server.requests.lock().unwrap();
    assert_eq!(requests.len(),2,"only Proposer and Selector are LLM stages");
    assert!(rendered_prompt(&requests[1]).contains("square_control_deltas"));
}

#[tokio::test]
async fn illegal_candidate_does_not_veto_legal_candidate_or_require_retry() {
    let server=Server::script(vec![
        text(json!({"candidates":[
            {"id":"bad","move":"Nxe4","counterplay":["Bh6"]},
            {"id":"good","move":"e5","counterplay":["Bb5"]}
        ]}))
    ],512);
    let mut m=state();
    let mv=llm_match::generate_move(&mut m,&cfg(&server),|_,_|panic!("unexpected retry")).await.unwrap();
    assert_eq!(mv.to_uci(),"e7e5");
    assert_eq!(server.requests.lock().unwrap().len(),1);
}

#[tokio::test]
async fn minimal_proposer_move_works_without_selector_or_counterplay() {
    let server=Server::script(vec![text(json!({"move":"Nf6"}))],512);
    let mut m=state();
    let mv=llm_match::generate_move(&mut m,&cfg(&server),|_,_|panic!()).await.unwrap();
    assert_eq!(mv.to_uci(),"g8f6");
    assert_eq!(server.requests.lock().unwrap().len(),1);
}

#[tokio::test]
async fn empty_then_valid_proposal_retries_without_board_mutation() {
    let server=Server::script(vec![
        text(json!({"candidates":[{"move":"Nxe4"}]})),
        text(json!({"move":"e5"}))
    ],512);
    let mut m=state();
    let original=m.game().position().to_fen();
    let mut rejected=0;
    let mv=llm_match::generate_move(&mut m,&cfg(&server),|_,_|{rejected+=1;}).await.unwrap();
    assert_eq!(mv.to_uci(),"e7e5");
    assert_eq!(rejected,1);
    assert_ne!(m.game().position().to_fen(),original);
    assert_eq!(server.requests.lock().unwrap().len(),2);
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
            "reasoning_content":"I should call piece_at for c3 before trusting the board. Perhaps piece_at is useful here.",
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
    assert_eq!(first,json!({"square":"c3","piece":"White Knight"}));
    assert_eq!(second,json!({"square":"c4","piece":null}));
    assert_eq!(server.prompts.lock().unwrap().len(),2); // one attempt, two generations
}

#[tokio::test]
async fn piece_at_names_are_readable_for_both_colors() {
    use llm_match::pipeline::{Stage,debug::{self,Inputs}};
    let fen="r3k2r/p1ppqpb1/bn2pnp1/2pP4/1p2P3/2N2N2/PPQBBPPP/R3K2R w KQkq - 0 1";
    let m=llm_match::Match::from_fen(Variant::Classic,Color::White,fen).unwrap();
    let q=llm_match::grounder::engine_map(m.game().position()).unwrap()
        .pieces.into_iter().find(|p|p.id=="wQ_c2").unwrap();
    let call=|id:&str,square:&str| json!({
        "id":id,"type":"function",
        "function":{"name":"piece_at","arguments":json!({"square":square}).to_string()}
    });
    let server=Server::script(vec![
        Reply::Message(json!({"role":"assistant","content":null,
            "tool_calls":[call("black-queen","e7"),call("white-queen","c2"),
                call("white-knight","c3"),call("empty-square","c4")]
        })),
        text(json!({"controls":q.controls,"xray":q.xray})),
    ],512);
    debug::run(&m,&cfg(&server),Stage::OneGrounder,&Inputs{
        selected_piece:Some("c2".into()),..Default::default()
    }).await.unwrap();
    let requests=server.requests.lock().unwrap();
    let observed:Vec<Value>=requests[1]["messages"].as_array().unwrap().iter()
        .filter(|m|m["role"]=="tool")
        .map(|m|serde_json::from_str(m["content"].as_str().unwrap()).unwrap())
        .collect();
    assert_eq!(observed,vec![
        json!({"square":"e7","piece":"Black Queen"}),
        json!({"square":"c2","piece":"White Queen"}),
        json!({"square":"c3","piece":"White Knight"}),
        json!({"square":"c4","piece":null})
    ]);
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
