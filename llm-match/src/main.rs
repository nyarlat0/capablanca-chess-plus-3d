use anyhow::{Context, Result, bail};
use capablanca_chess_plus::{Color, GameOutcome, Variant};
use std::io::{self, Write};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut config = llm_match::Config::load()?;
    let mut variant = Variant::Gothic;
    let mut human = Color::White;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
                    "llm-match [--variant Gothic|Embassy|Grand|Shako|Pemba|TerachessII] [--black] [--url http://aistation:5001]\nEnter a UCI move, or quit. Defaults come from llm-match/.env."
                );
                return Ok(());
            }
            _ => bail!("unknown argument {arg}"),
        }
    }
    let mut game = llm_match::Match::new(variant, human.opposite());
    loop {
        println!("{}", llm_match::board_state(game.game().position()));
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
