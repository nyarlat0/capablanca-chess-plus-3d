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
                        if request.get("messages").is_none() {
                            let response = json!({"value": 42}).to_string();
                            let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response);
                            continue;
                        }
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
fn grounding() -> Reply {
    Reply::Text("<thinking>GROUNDER_PRIVATE_REASONING</thinking>GROUNDED_FACT".into())
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
    c.pipeline_mode = llm_match::pipeline::two_stage::PipelineMode::Legacy;
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

fn two_cfg(server: &Server) -> llm_match::Config {
    let mut c = cfg(server);
    c.pipeline_mode = llm_match::pipeline::two_stage::PipelineMode::TwoStage;
    c
}

#[tokio::test]
async fn four_stage_skips_grounder_and_keeps_calculations_independent() {
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
    c.pipeline_mode = "four-stage".parse().unwrap();
    c.pipeline.grounder.content = "GROUNDER_MUST_NOT_RUN".into();
    c.pipeline.analyst.content = "ANALYST_ONLY".into();
    c.pipeline.proposer.content = "PROPOSER_ONLY".into();
    c.pipeline.verifier.content = "VERIFIER_ONLY".into();
    c.pipeline.selector.content = "SELECTOR_ONLY".into();
    let mut m = state();
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 6);
    for (index, marker) in [
        (0, "ANALYST_ONLY"),
        (1, "PROPOSER_ONLY"),
        (2, "VERIFIER_ONLY"),
        (3, "VERIFIER_ONLY"),
        (4, "VERIFIER_ONLY"),
        (5, "SELECTOR_ONLY"),
    ] {
        assert!(p[index].contains(marker));
    }
    assert!(
        p.iter()
            .all(|prompt| !prompt.contains("GROUNDER_MUST_NOT_RUN"))
    );
    assert!(p[5].contains("tactical_result"));
    assert!(!p[2].contains("rationale"));
    assert!(!p[5].contains("rationale"));
}
fn ranked(moves: &[&str]) -> Reply {
    text(
        json!({"candidates":moves.iter().map(|mv| json!({"move":mv,"rationale":"PRIVATE_PROPOSER_IDEA"})).collect::<Vec<_>>()}),
    )
}
fn choice(id: &str) -> Reply {
    text(json!({"choice":id}))
}

#[tokio::test]
async fn single_stage_commits_first_legal_candidate_with_one_generation() {
    for (moves, expected) in [
        (vec!["Nf6", "e5", "d5"], "g8f6"),
        (vec!["e6+", "Nf6", "e5"], "g8f6"),
        (vec!["e6+", "Qh4", "e5"], "e7e5"),
    ] {
        let server = Server::script(vec![ranked(&moves)], 512);
        let mut c = two_cfg(&server);
        c.pipeline_mode = llm_match::pipeline::two_stage::PipelineMode::SingleStage;
        c.two_stage.selector.template.content = "SELECTOR_MUST_NOT_RUN".into();
        let mut m = state();
        llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .unwrap();
        assert_eq!(m.uci_history(), ["e2e4", expected]);
        assert!(m.wrong_moves().is_empty());
        let prompts = server.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(!prompts[0].contains("SELECTOR_MUST_NOT_RUN"));
    }
}

#[tokio::test]
async fn single_stage_retries_only_proposer_when_all_candidates_are_illegal() {
    let server = Server::script(
        vec![ranked(&["e6+", "Qh4", "Ke7"]), ranked(&["e6+", "e5", "d5"])],
        512,
    );
    let mut c = two_cfg(&server);
    c.pipeline_mode = llm_match::pipeline::two_stage::PipelineMode::SingleStage;
    c.two_stage.proposer.template.user_prompt.push_str(
        "{{#if excluded-moves}}EXCLUSIONS_PRESENT {{excluded-moves}}{{else}}EXCLUSIONS_EMPTY{{/if}}",
    );
    let mut m = state();
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 2);
    assert!(p[0].contains("EXCLUSIONS_EMPTY"));
    assert!(!p[0].contains("EXCLUSIONS_PRESENT"));
    assert!(p[1].contains("EXCLUSIONS_PRESENT"));
    assert!(!p[1].contains("EXCLUSIONS_EMPTY"));
    assert!(p[1].contains("Qh4"));
    assert!(m.wrong_moves().is_empty());
}

#[tokio::test]
async fn two_stage_success_is_exactly_two_generations_and_selector_is_independent() {
    let server = Server::script(
        vec![
            ranked(&["e5", "d5", "Nf6"]),
            Reply::Text("<thinking>private selector</thinking>{\"choice\":\"c2\"}".into()),
        ],
        512,
    );
    let mut m = state();
    let mut c = two_cfg(&server);
    c.two_stage.proposer.template.content = "CUSTOM_PROPOSER {REASON:custom-proposer}".into();
    c.two_stage.selector.template.content = "CUSTOM_SELECTOR {REASON:custom-selector}".into();
    c.two_stage.selector.reasoning.prefix = "<thinking>".into();
    c.two_stage.selector.reasoning.suffix = "</thinking>".into();
    c.two_stage.proposer.preset =
        prompt_core::Preset::from_value(json!({"genamt":1000,"temp":0.8})).unwrap();
    c.two_stage.selector.preset =
        prompt_core::Preset::from_value(json!({"genamt":2000,"temp":0.6})).unwrap();
    llm_match::generate_move(&mut m, &c, |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "d7d5"]);
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 2);
    assert!(p[0].contains("CUSTOM_PROPOSER {REASON:custom-proposer}"));
    assert!(p[1].contains("CUSTOM_SELECTOR {REASON:custom-selector}"));
    assert!(!p[1].contains("PRIVATE_PROPOSER_IDEA"));
    assert!(!p[1].contains("rationale"));
    assert!(!p[1].contains("1. e4"));
    assert!(p[1].contains(r#"{"id":"c1","move":"e5"}"#));
    assert!(p[1].contains(r#"{"id":"c2","move":"d5"}"#));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests[0]["max_tokens"], 1000);
    assert_eq!(requests[1]["max_tokens"], 2000);
    assert_eq!(requests[1]["messages"].as_array().unwrap().len(), 2);
    for mv in state().game().position().legal_moves() {
        assert!(p.iter().all(|s| !s.contains(&mv.to_uci())));
    }
}

#[tokio::test]
async fn two_stage_preserves_preference_order_after_illegal_and_duplicate_filtering() {
    let server = Server::script(
        vec![ranked(&["e6+", "Nf6", "e5", "Nf6", "d5"]), choice("c2")],
        512,
    );
    let mut m = state();
    llm_match::generate_move(&mut m, &two_cfg(&server), |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]); // Selector may choose a lower-ranked move.
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 2);
    assert!(
        p[1].contains(
            r#"[{"id":"c1","move":"Nf6"},{"id":"c2","move":"e5"},{"id":"c3","move":"d5"}]"#
        )
    );
    assert!(!p[1].contains("e6+"));
    assert!(!p[1].contains("PRIVATE_PROPOSER_IDEA"));
    assert!(!p[1].contains("rationale"));
}

#[tokio::test]
async fn two_stage_repairs_syntax_locally_without_regeneration() {
    let server = Server::script(vec![
        Reply::Text("Preface <think>PRIVATE_THINK</think>PROPOSER_EXTRA_TEXT\n```json\n{\"candidates\":[{\"move\":\"e5\",\"rationale\":\"A,} string\",},{\"move\":\"d5\",\"rationale\":\"B\"},{\"move\":\"Nf6\",\"rationale\":\"C\"},],}\n```".into()),
        Reply::Text("Preface <think>private</think>SELECTOR_EXTRA_TEXT\n```json\n{\"choice\":\"c2\",}".into()),
    ], 512);
    let mut m = state();
    llm_match::generate_move(&mut m, &two_cfg(&server), |_, _| panic!())
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 2);
    assert!(!p[1].contains("PRIVATE_THINK"));
    assert!(!p[1].contains("PROPOSER_EXTRA_TEXT"));
    assert_eq!(m.uci_history(), ["e2e4", "d7d5"]);
}

#[tokio::test]
async fn two_stage_accepts_final_plain_json_after_arbitrary_text() {
    let Reply::Text(proposal) = ranked(&["e5", "d5", "Nf6"]) else {
        unreachable!()
    };
    let server = Server::script(
        vec![
            Reply::Text(format!(
                "My observations and an earlier {{\"draft\":true}}.\n{proposal}"
            )),
            Reply::Text("I prefer the second candidate.\n{\"choice\":\"c2\"}".into()),
        ],
        512,
    );
    let mut m = state();
    llm_match::generate_move(&mut m, &two_cfg(&server), |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "d7d5"]);
    assert_eq!(server.prompts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn two_stage_filters_illegal_and_duplicates_and_retries_only_selector() {
    let server = Server::script(
        vec![
            ranked(&["e6+", "e5", "e5"]),
            choice("missing"),
            choice("c1"),
        ],
        512,
    );
    let mut m = state();
    llm_match::generate_move(&mut m, &two_cfg(&server), |_, _| panic!())
        .await
        .unwrap();
    assert_eq!(m.uci_history(), ["e2e4", "e7e5"]);
    let p = server.prompts.lock().unwrap();
    assert_eq!(p.len(), 3);
    assert_eq!(p[1], p[2]);
    assert!(!p[1].contains("e6+"));
    assert!(!p[1].contains(r#""id":"c2""#));
}

#[tokio::test]
async fn two_stage_reject_all_and_no_legal_candidates_retry_with_exclusions() {
    for first in [ranked(&["e6+", "Qh4", "Ke7"]), ranked(&["e5", "d5", "Nf6"])] {
        let first_is_legal = matches!(&first, Reply::Text(s) if s.contains("Nf6"));
        let mut replies = vec![first];
        if first_is_legal {
            replies.push(choice("reject_all"));
        }
        replies.extend([ranked(&["e6", "d6", "Nc6"]), choice("c1")]);
        let server = Server::script(replies, 512);
        let mut m = state();
        llm_match::generate_move(&mut m, &two_cfg(&server), |_, _| panic!())
            .await
            .unwrap();
        assert_eq!(m.uci_history(), ["e2e4", "e7e6"]);
        assert!(m.wrong_moves().is_empty());
        let p = server.prompts.lock().unwrap();
        assert_eq!(p.len(), if first_is_legal { 4 } else { 3 });
        let next = &p[if first_is_legal { 2 } else { 1 }];
        assert!(next.contains(if first_is_legal { "Nf6" } else { "Qh4" }));
    }
}

#[tokio::test]
async fn two_stage_all_variants_and_fairy_representations_use_only_two_generations() {
    use llm_match::OutputFormat;
    for variant in Variant::ALL {
        for profile in if variant == Variant::Classic {
            vec!["classic"]
        } else {
            vec![
                "classic",
                "numeric-no-history",
                "numeric-ascii",
                "hybrid",
                "uci-no-history",
                "numeric-json-no-history",
            ]
        } {
            let mut c = if variant == Variant::Classic {
                llm_match::Config::load_classic_with_profile(Some("no-history"))
            } else {
                llm_match::Config::load_with_profile(Some(profile))
            }
            .unwrap();
            c.pipeline_mode = llm_match::pipeline::two_stage::PipelineMode::TwoStage;
            let mut m = llm_match::Match::new(variant, Color::White);
            let mut moves: Vec<_> = m
                .game()
                .position()
                .legal_moves()
                .into_iter()
                .take(3)
                .collect();
            moves.sort_by_key(|mv| mv.to_uci());
            let candidates: Vec<_> = moves.iter().map(|mv| {
                let mut trial = m.clone(); trial.accept_model(&mv.to_uci()).unwrap();
                let value = if variant == Variant::Classic { json!(trial.classic_san_history().strip_prefix("1. ").unwrap()) }
                else { match c.representation.output_format {
                    OutputFormat::Semantic => json!(trial.history()[0].content),
                    OutputFormat::Numeric => json!(trial.numeric_history()[0].content),
                    OutputFormat::Uci => json!(mv.to_uci()),
                    OutputFormat::NumericJson => json!({"from":llm_match::numeric_square(mv.from),"to":llm_match::numeric_square(mv.to),"promotion":mv.promotion.map(|p|p.display_name())}),
                }};
                json!({"move":value,"rationale":"PRIVATE_IDEA"})
            }).collect();
            let server = Server::script(
                vec![text(json!({"candidates":candidates})), choice("c1")],
                512,
            );
            c.endpoint = server.endpoint.clone();
            llm_match::generate_move(&mut m, &c, |_, _| panic!())
                .await
                .unwrap();
            assert_eq!(m.uci_history(), [moves[0].to_uci()]);
            let p = server.prompts.lock().unwrap();
            assert_eq!(p.len(), 2);
            assert!(!p[1].contains("PRIVATE_IDEA"));
            if variant != Variant::Classic {
                assert!(p.iter().all(|s| !s.contains("FEN:")));
                if c.representation.state_format == llm_match::StateFormat::Numeric {
                    assert!(
                        p.iter()
                            .all(|s| !s.contains("ASCII BOARD") && !s.contains("a b c"))
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn two_stage_exhausted_rounds_preserve_board_and_history() {
    let server = Server::script(
        vec![
            ranked(&["e5", "d5", "Nf6"]),
            choice("reject_all"),
            ranked(&["e5", "d5", "Nf6"]),
        ],
        512,
    );
    let mut m = state();
    let before = m.game().position().to_fen();
    llm_match::generate_move(&mut m, &two_cfg(&server), |_, _| panic!())
        .await
        .unwrap_err();
    assert_eq!(m.game().position().to_fen(), before);
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert!(m.wrong_moves().is_empty());
    assert_eq!(server.prompts.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn two_stage_reasoning_and_transport_retries_are_silent() {
    let server = Server::script(
        vec![
            Reply::Text("Preface <think>unfinished {\"candidates\":[]}".into()),
            Reply::Http(503),
            ranked(&["e5", "d5", "Nf6"]),
            Reply::Text("Preface <think>{\"choice\":\"c1\"}</think>".into()),
            choice("c2"),
        ],
        512,
    );
    let mut m = state();
    llm_match::generate_move(&mut m, &two_cfg(&server), |_, _| panic!())
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert_eq!(p[0], p[1]);
    assert_eq!(p[1], p[2]);
    assert_eq!(p[3], p[4]);
    assert!(p.iter().all(|s| !s.contains("unfinished")));
    assert!(m.wrong_moves().is_empty());
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
            grounding(),
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
    assert_eq!(server.prompts.lock().unwrap().len(), 7);
}

#[tokio::test]
async fn each_stage_sends_its_own_sampler_and_response_budget() {
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
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
        analyst: preset(101, 0.1),
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
    assert_eq!(requests.len(), 7);
    for (request, (tokens, temperature)) in requests.iter().zip([
        (101, 0.1),
        (151, 0.15),
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
async fn independent_stages_commit_only_selected_move() {
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
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
    assert_eq!(p.len(), 7);
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
    assert_eq!(requests[1]["messages"].as_array().unwrap().len(), 2);
    assert!(p[1].contains("ANALYST_PRIVATE_FACT"));
    assert!(p[1].contains("Any format, including {braces}, is fine."));
    assert!(p.iter().all(|prompt| !prompt.contains("PRIVATE_THINKING")));
    assert!(p[2].contains("GROUNDED_FACT"));
    assert!(!p[2].contains("ANALYST_PRIVATE_FACT"));
    assert!(!p[2].contains("GROUNDER_PRIVATE_REASONING"));
    assert!(!p[1].contains("1. e4"));
    let initial = state().game().position().to_fen();
    for prompt in p.iter() {
        assert!(prompt.contains(&initial));
        assert!(prompt.contains("ASCII BOARD"));
    }
    for prompt in &p[3..] {
        assert!(!prompt.contains("ANALYST_PRIVATE_FACT"));
        assert!(!prompt.contains("GROUNDED_FACT"));
        assert!(!prompt.contains("PROPOSER_PRIVATE_RATIONALE"));
        assert!(!prompt.contains("1. e4"));
    }
    assert!(p[3].contains("\"move\":\"e5\""));
    assert!(!p[3].contains("\"move\":\"d5\""));
    assert!(!p[3].contains("\"move\":\"Nf6\""));
    assert!(!p[6].contains("\"move\":\"d5\""));
    assert!(!p[6].contains("TACTICAL_RESULT_2"));
    assert!(p[6].contains(r#""tactical_result":"TACTICAL_RESULT_1""#));
    assert!(p[6].contains(r#""tactical_result":"TACTICAL_RESULT_3""#));
    assert!(!p[6].contains("verifier_note"));
    for prompt in &p[3..6] {
        assert!(!prompt.contains(r#""id""#));
        assert!(!prompt.contains("candidate_id"));
    }
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
            grounding(),
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
            grounding(),
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
    assert_eq!(p[4], p[5]);
    assert_eq!(p[5], p[6]);
    for prompt in p.iter() {
        assert!(!prompt.contains("unfinished"));
        assert!(!prompt.contains("bad JSON"));
    }
}
#[tokio::test]
async fn grounder_retries_are_silent_and_do_not_rerun_analyst() {
    let server = Server::script(
        vec![
            analysis(),
            Reply::Text("<thinking>unfinished grounder".into()),
            Reply::Http(429),
            grounding(),
            normal_proposal(),
            verify(1, true),
            verify(2, false),
            verify(3, false),
            select(1, "e5"),
        ],
        512,
    );
    let mut m = state();
    llm_match::generate_move(&mut m, &cfg(&server), |_, _| panic!())
        .await
        .unwrap();
    let p = server.prompts.lock().unwrap();
    assert_eq!(p[1], p[2]);
    assert_eq!(p[2], p[3]);
    assert!(p[4].contains("GROUNDED_FACT"));
    assert!(!p[4].contains("ANALYST_PRIVATE_FACT"));
    assert!(p.iter().all(|s| !s.contains("unfinished grounder")));
    assert!(m.wrong_moves().is_empty());
}

#[tokio::test]
async fn engine_gate_filters_illegal_and_mismatched_candidates_without_committing() {
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
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
    assert_eq!(p.len(), 7);
    assert!(p[3].contains("e6+"));
    assert!(p[3].contains("Qh4"));
    assert!(p[3].contains("GROUNDED_FACT"));
    assert!(!p[3].contains("ANALYST_PRIVATE_FACT"));
    assert!(!p[4].contains("e6+"));
    assert!(p[4].contains(r#""move":"e5""#));
    assert!(p[5].contains(r#""move":"d5""#));
}

#[tokio::test]
async fn incorrect_metadata_cannot_poison_same_legal_move_and_valid_duplicates_are_deduplicated() {
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
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
    assert_eq!(p.len(), 5); // only one verifier call, for the first valid candidate
    assert!(p[3].contains(r#""from":"f6""#));
    assert!(!p[3].contains(r#""from":"c6""#));
    assert!(!p[3].contains("candidate_id"));
    assert!(!p[3].contains(r#""id""#));
    assert!(p[4].contains(r#""id":"c2""#));
    assert!(p[4].contains(r#""tactical_result":"TACTICAL_RESULT_2""#));
    assert!(!p[4].contains(r#""id":"c1""#));
    assert!(!p[4].contains(r#""id":"c3""#));
}

#[tokio::test]
async fn exhausted_gate_does_not_mutate_position_history_or_rejected_finals() {
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
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
    assert_eq!(server.prompts.lock().unwrap().len(), 3);
}
#[tokio::test]
async fn all_rejected_is_bounded_and_selector_cannot_invent_move() {
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
            normal_proposal(),
            verify(1, false),
            verify(2, false),
            verify(3, false),
            normal_proposal(),
        ],
        512,
    );
    let mut m = state();
    let mut c = cfg(&server);
    c.pipeline.proposer.user_prompt.push_str(
        "{{#if excluded-moves}}EXCLUSIONS_PRESENT {{excluded-moves}}{{else}}EXCLUSIONS_EMPTY{{/if}}",
    );
    assert!(
        llm_match::generate_move(&mut m, &c, |_, _| panic!())
            .await
            .is_err()
    );
    assert_eq!(server.prompts.lock().unwrap().len(), 7);
    {
        let prompts = server.prompts.lock().unwrap();
        assert!(prompts[2].contains("EXCLUSIONS_EMPTY"));
        assert!(!prompts[2].contains("EXCLUSIONS_PRESENT"));
        assert!(prompts[6].contains("EXCLUSIONS_PRESENT"));
        assert!(!prompts[6].contains("EXCLUSIONS_EMPTY"));
    }
    assert!(m.wrong_moves().is_empty());
    assert_eq!(m.uci_history(), ["e2e4"]);
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
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
    assert_eq!(p[6], p[7]);
    assert_eq!(p[7], p[8]);
}
#[tokio::test]
async fn malformed_verifier_is_not_a_rejection() {
    let server = Server::script(
        vec![
            analysis(),
            grounding(),
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
    assert_eq!(p[3], p[4]);
    assert_eq!(p[4], p[5]);
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
                grounding(),
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
            c.pipeline_mode = llm_match::pipeline::two_stage::PipelineMode::Legacy;
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
                    grounding(),
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
    let server = Server::script(vec![analysis(), grounding(), invalid], 512);
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
            grounding(),
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
    for prompt in &p[3..] {
        assert!(prompt.contains("PROBE history= last= wrong= analysis= excluded= ENDPROBE"));
        assert!(!prompt.contains("e6+"));
    }
}
