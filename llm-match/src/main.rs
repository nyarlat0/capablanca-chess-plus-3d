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
    let mut config = llm_match::Config::load()?;
    let mut variant = Variant::Gothic;
    let mut human = Color::White;
    let mut print_prompt = false;
    let mut replay = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--llm-profile" => {
                args.next().context("missing profile name")?;
            }
            s if s.starts_with("--llm-profile=") => {}
            "--print-prompt" => print_prompt = true,
            "--moves" => replay = args.next().context("missing quoted UCI history")?,
            "--url" => {
                config.endpoint =
                    llm_match::normalize_endpoint(&args.next().context("missing URL")?)?
            }
            "--black" => human = Color::Black,
            "--variant" => {
                let name = args.next().context("missing variant")?.to_ascii_lowercase();
                variant = Variant::ALL.into_iter().find(|v| format!("{v:?}").to_ascii_lowercase() == name).context("unknown variant (use Gothic, Embassy, Grand, Shako, Pemba, TerachessII, etc.)")?;
            }
            "--help" => {
                println!(
                    "llm-match [--variant Gothic|Embassy|Grand|Shako|Pemba|TerachessII] [--black] [--url http://aistation:5001] [--llm-profile NAME] [--print-prompt] [--moves 'e2e4 e7e5']\nEnter a UCI move, or quit. Defaults come from llm-match/.env."
                );
                return Ok(());
            }
            _ => bail!("unknown argument {arg}"),
        }
    }
    let mut game = llm_match::Match::new(variant, human.opposite());
    eprintln!(
        "LLM representation profile: {}\n{}",
        config.profile_name,
        config.representation.summary()
    );
    for mv in replay.split_whitespace() {
        if game.game().position().side_to_move() == human {
            game.accept_human(mv)?;
        } else {
            game.accept_model(mv)?;
        }
    }
    if print_prompt {
        let data = game.prompt_data_with(&config)?;
        println!(
            "{}",
            prompt_core::PromptBuilder::new()
                .build(&config.context, &config.instruct, &config.system, &data)?
                .text
        );
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
