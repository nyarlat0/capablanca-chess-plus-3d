use anyhow::{Context, Result, bail};
use capablanca_chess_plus::{Color, GameOutcome, Variant};
use std::io::{self, Write};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "llm_match=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let mut endpoint = None;
    let mut variant = Variant::Classic;
    let mut human = Color::White;
    let mut print_prompt = false;
    let mut replay = String::new();
    let mut moves_format = String::from("uci");
    let mut initial_fen = None;
    let mut stage: Option<llm_match::pipeline::Stage> = None;
    let mut map_file = None;
    let mut inspect_square = None;
    let mut inspect_piece = None;
    let mut inputs = llm_match::pipeline::debug::Inputs::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--llm-profile" | "--llm-classic-profile" => {
                args.next().context("missing profile name")?;
            }
            s if s.starts_with("--llm-profile=") || s.starts_with("--llm-classic-profile=") => {}
            "--print-prompt" => print_prompt = true,
            "--stage" => stage = Some(args.next().context("missing stage name")?.parse()?),
            "--grounder-map-file" => map_file = Some(args.next().context("missing map file")?),
            "--inspect-square" => inspect_square = Some(args.next().context("missing square")?),
            "--inspect-piece" => inspect_piece = Some(args.next().context("missing piece id")?),
            "--analysis-file" => {
                inputs.analysis = Some(std::fs::read_to_string(
                    args.next().context("missing analysis file")?,
                )?)
            }
            "--candidate-file" => {
                inputs.candidate = Some(serde_json::from_str(&std::fs::read_to_string(
                    args.next().context("missing candidate file")?,
                )?)?)
            }
            "--accepted-candidates-file" => {
                inputs.accepted_candidates = Some(serde_json::from_str(&std::fs::read_to_string(
                    args.next().context("missing accepted candidates file")?,
                )?)?)
            }
            "--fen" => initial_fen = Some(args.next().context("missing quoted initial FEN")?),
            s if s.starts_with("--fen=") => initial_fen = Some(s[6..].to_owned()),
            "--moves" => replay = args.next().context("missing quoted move history")?,
            "--moves-format" => {
                moves_format = args.next().context("missing moves format: uci or san")?
            }
            "--url" => {
                endpoint = Some(llm_match::normalize_endpoint(
                    &args.next().context("missing URL")?,
                )?)
            }
            "--black" => human = Color::Black,
            "--variant" => {
                let name = args.next().context("missing variant")?.to_ascii_lowercase();
                variant = Variant::ALL.into_iter().find(|v| format!("{v:?}").to_ascii_lowercase() == name).context("unknown variant (use Gothic, Embassy, Grand, Shako, Pemba, TerachessII, etc.)")?;
            }
            "--help" => {
                println!(
                    "Grounder map (Classic): --stage grounder --grounder-map-file map.json [--inspect-square e4] [--inspect-piece bN_f6]. Offline validation against --fen/--moves; no LLM configuration or network required. Without inspect flags prints normalized map JSON."
                );
                println!(
                    "Stage debug: --stage grounder|proposer|verifier|selector [--analysis-file facts.txt] [--candidate-file candidate.json] [--accepted-candidates-file accepted.json]. Runs only the chosen stage, without committing a move. --print-prompt previews it offline. Default variant: Classic."
                );
                println!(
                    "llm-match [--variant Classic|Gothic|Embassy|Grand|Shako|Pemba|TerachessII] [--black] [--url http://aistation:5001] [--llm-profile NAME] [--llm-classic-profile with-history|no-history] [--fen 'FEN'] [--print-prompt] [--moves 'e2e4 e7e5']\nEnter a UCI move, or quit. Defaults come from llm-match/.env. Classic chess uses its separate SAN configuration."
                );
                println!(
                    "--moves-format uci|san: format of --moves (default uci; san requires Classic). Moves are replayed from --fen or the initial position; SAN input is whitespace-separated moves, without PGN headers or move numbers."
                );
                return Ok(());
            }
            _ => bail!("unknown argument {arg}"),
        }
    }
    anyhow::ensure!(
        matches!(moves_format.as_str(), "uci" | "san"),
        "--moves-format must be uci or san"
    );
    anyhow::ensure!(
        moves_format != "san" || variant == Variant::Classic,
        "SAN replay requires --variant Classic"
    );
    anyhow::ensure!(
        map_file.is_some() || (inspect_square.is_none() && inspect_piece.is_none()),
        "inspection requires --grounder-map-file"
    );
    if map_file.is_some() {
        anyhow::ensure!(
            stage == Some(llm_match::pipeline::Stage::Grounder) && variant == Variant::Classic,
            "--grounder-map-file requires --stage grounder and Classic"
        );
        anyhow::ensure!(
            !print_prompt
                && inputs.analysis.is_none()
                && inputs.candidate.is_none()
                && inputs.accepted_candidates.is_none(),
            "map loading cannot be combined with --print-prompt or other stage input files"
        );
    }
    let mut game = match initial_fen {
        Some(fen) => llm_match::Match::from_fen(variant, human.opposite(), &fen)
            .context("invalid initial FEN for selected variant")?,
        None => llm_match::Match::new(variant, human.opposite()),
    };
    for (index, text) in replay.split_whitespace().enumerate() {
        let decoded = if moves_format == "san" {
            game.game()
                .position()
                .parse_san_move(text)
                .with_context(|| format!("invalid SAN history move {}: {text}", index + 1))?
                .to_uci()
        } else {
            text.to_owned()
        };
        let mv = decoded.as_str();
        if game.game().position().side_to_move() == human {
            game.accept_human(mv)?;
        } else {
            game.accept_model(mv)?;
        }
    }
    if let Some(path) = map_file {
        let raw = std::fs::read_to_string(&path).with_context(|| format!("cannot read {path}"))?;
        let graph = llm_match::grounder::ControlGraph::parse(game.game().position(), &raw)
            .with_context(|| format!("invalid Grounder map {path}\n--- original map ---\n{raw}"))?;
        if inspect_square.is_none() && inspect_piece.is_none() {
            println!("{}", graph.normalized_json()?);
        } else {
            let mut result = serde_json::Map::new();
            if let Some(cell) = inspect_square {
                result.insert(
                    "square".into(),
                    serde_json::to_value(graph.get_square(&cell)?)?,
                );
            }
            if let Some(id) = inspect_piece {
                result.insert(
                    "piece".into(),
                    serde_json::to_value(
                        graph
                            .get_piece(&id)
                            .with_context(|| format!("unknown piece id {id}"))?,
                    )?,
                );
            }
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        return Ok(());
    }
    let mut config = llm_match::Config::load_for_variant(variant)?;
    if let Some(endpoint) = endpoint {
        config.endpoint = endpoint;
    }
    eprintln!(
        "LLM representation profile: {}\n{}",
        config.profile_name,
        config.summary()
    );
    if let Some(stage) = stage {
        let result = if print_prompt {
            llm_match::pipeline::debug::preview(&game, &config, stage, &inputs)?
        } else {
            llm_match::pipeline::debug::run(&game, &config, stage, &inputs).await?
        };
        println!("{result}");
        return Ok(());
    }
    anyhow::ensure!(
        inputs.analysis.is_none()
            && inputs.candidate.is_none()
            && inputs.accepted_candidates.is_none(),
        "stage input files require --stage"
    );
    if print_prompt {
        println!("{}", llm_match::pipeline::preview(&game, &config)?);
        return Ok(());
    }
    loop {
        println!(
            "{}",
            llm_match::render_state(game.game().position(), config.representation.state_format)
        );
        if !matches!(
            game.game().outcome(),
            GameOutcome::Ongoing | GameOutcome::Check
        ) {
            println!("{:?}", game.game().outcome());
            break;
        }
        if game.game().position().side_to_move() == human {
            let Some(line) = read_line("Your move (or quit): ")? else {
                break;
            };
            if let Err(error) = game.accept_human(&line) {
                eprintln!("{error}");
            }
        } else {
            match llm_match::generate_move(&mut game, &config, |n, text| {
                eprintln!("Rejected attempt {n}: {text:?}")
            })
            .await
            {
                Ok(mv) => println!("LLM: {}", mv.to_uci()),
                Err(error) => {
                    eprintln!("{error:#}");
                    if read_line("Enter to retry unchanged position, or quit: ")?.is_none() {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

fn read_line(prompt: &str) -> Result<Option<String>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 || line.trim() == "quit" {
        Ok(None)
    } else {
        Ok(Some(line))
    }
}
