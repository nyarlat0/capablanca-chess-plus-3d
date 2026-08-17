use capablanca_chess_plus::PieceKind;
use std::env;
use std::process::ExitCode;
use terastockfish::EvaluationParameters;
use terastockfish::validation::{
    CandidateResult, GameTermination, SelfPlayConfig, compare_with_published,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terastockfish-eval: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut config = SelfPlayConfig::default();
    let mut candidate = EvaluationParameters::published();
    let mut changes = Vec::new();
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            "--smoke" => config = SelfPlayConfig::smoke(),
            "--min-pairs" => config.minimum_pairs = parse_next(&mut arguments, "--min-pairs")?,
            "--max-pairs" => config.maximum_pairs = parse_next(&mut arguments, "--max-pairs")?,
            "--nodes" => config.nodes_per_move = parse_next(&mut arguments, "--nodes")?,
            "--max-plies" => config.maximum_plies = parse_next(&mut arguments, "--max-plies")?,
            "--opening-plies" => {
                config.opening_plies = parse_next(&mut arguments, "--opening-plies")?
            }
            "--hash" => config.hash_megabytes = parse_next(&mut arguments, "--hash")?,
            "--jobs" => config.jobs = parse_next(&mut arguments, "--jobs")?,
            "--adjudication-score" => {
                config.adjudication_score = parse_next(&mut arguments, "--adjudication-score")?
            }
            "--adjudication-plies" => {
                config.adjudication_plies = parse_next(&mut arguments, "--adjudication-plies")?
            }
            "--seed" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--seed requires a value".to_owned())?;
                config.seed = parse_seed(&value)?;
            }
            "--set" => {
                let assignment = arguments
                    .next()
                    .ok_or_else(|| "--set requires NAME=VALUE".to_owned())?;
                apply_assignment(&mut candidate, &assignment)?;
                changes.push(assignment);
            }
            _ => return Err(format!("unknown argument `{argument}`; use --help")),
        }
    }

    println!("# baseline=published");
    println!("# candidate_changes={}", changes.join(";"));
    println!(
        "# min_pairs={} max_pairs={} nodes_per_move={} max_plies={} opening_plies={} hash_mb={} jobs={} adjudication_score={} adjudication_plies={} seed={}",
        config.minimum_pairs,
        config.maximum_pairs,
        config.nodes_per_move,
        config.maximum_plies,
        config.opening_plies,
        config.hash_megabytes,
        config.jobs,
        config.adjudication_score,
        config.adjudication_plies,
        config.seed
    );
    let summary = compare_with_published(candidate, config).map_err(|error| error.to_string())?;
    println!("pair,candidate_color,result,termination,plies,nodes,elapsed_ms,opening_fen");
    for game in &summary.games {
        println!(
            "{},{},{},{},{},{},{},{}",
            game.pair,
            color_name(game.candidate_color),
            result_name(game.result),
            termination_name(game.termination),
            game.plies,
            game.nodes,
            game.elapsed.as_millis(),
            game.opening_fen
        );
    }
    let (lower, upper) = summary.confidence_interval_95();
    println!(
        "# summary games={} wins={} draws={} losses={} score={:.4} ci95_lower={lower:.4} ci95_upper={upper:.4} decisive={} candidate_stronger={}",
        summary.games.len(),
        summary.wins(),
        summary.draws(),
        summary.losses(),
        summary.score(),
        summary.statistically_decisive,
        summary.candidate_is_stronger()
    );
    Ok(())
}

fn parse_next<T>(arguments: &mut impl Iterator<Item = String>, name: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    arguments
        .next()
        .ok_or_else(|| format!("{name} requires a value"))?
        .parse()
        .map_err(|_| format!("invalid value for {name}"))
}

fn parse_seed(value: &str) -> Result<u64, String> {
    value
        .strip_prefix("0x")
        .map_or_else(|| value.parse(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|_| format!("invalid seed `{value}`"))
}

fn apply_assignment(parameters: &mut EvaluationParameters, assignment: &str) -> Result<(), String> {
    let (name, value) = assignment
        .split_once('=')
        .ok_or_else(|| format!("invalid assignment `{assignment}`; expected NAME=VALUE"))?;
    let value = value
        .parse::<i32>()
        .map_err(|_| format!("invalid integer in `{assignment}`"))?;
    let name = name.to_ascii_lowercase().replace('-', "_");
    if let Some(piece) = name.strip_prefix("material.") {
        parameters.set_material_value(parse_piece_kind(piece)?, value);
    } else if let Some(piece) = name.strip_prefix("centrality.") {
        parameters.set_centrality_weight(parse_piece_kind(piece)?, value);
    } else if let Some(piece) = name.strip_prefix("advancement.") {
        parameters.set_advancement_weight(parse_piece_kind(piece)?, value);
    } else {
        match name.as_str() {
            "doubled_pawn" => parameters.set_doubled_pawn_penalty(value),
            "isolated_pawn" => parameters.set_isolated_pawn_penalty(value),
            "bishop_pair" => parameters.set_bishop_pair_bonus(value),
            "king_shelter" => parameters.set_king_shelter_bonus(value),
            "king_jump" => parameters.set_king_jump_bonus(value),
            "tempo" => parameters.set_tempo_bonus(value),
            _ => return Err(format!("unknown evaluation parameter `{name}`")),
        }
    }
    Ok(())
}

fn parse_piece_kind(name: &str) -> Result<PieceKind, String> {
    let piece = match name {
        "pawn" => PieceKind::Pawn,
        "knight" => PieceKind::Knight,
        "bishop" => PieceKind::Bishop,
        "rook" => PieceKind::Rook,
        "queen" => PieceKind::Queen,
        "king" => PieceKind::King,
        "archbishop" => PieceKind::Archbishop,
        "chancellor" => PieceKind::Chancellor,
        "cannon" => PieceKind::Cannon,
        "elephant" => PieceKind::Elephant,
        "camel" => PieceKind::Camel,
        "giraffe" => PieceKind::Giraffe,
        "archer" => PieceKind::Archer,
        "machine" => PieceKind::Machine,
        "amazon" => PieceKind::Amazon,
        "lion" => PieceKind::Lion,
        "buffalo" => PieceKind::Buffalo,
        "centaur" => PieceKind::Centaur,
        "admiral" => PieceKind::Admiral,
        "missionary" => PieceKind::Missionary,
        "eagle" => PieceKind::Eagle,
        "rhinoceros" => PieceKind::Rhinoceros,
        "prince" => PieceKind::Prince,
        "sorceress" => PieceKind::Sorceress,
        "duchess" => PieceKind::Duchess,
        "troll" => PieceKind::Troll,
        _ => return Err(format!("unknown piece kind `{name}`")),
    };
    Ok(piece)
}

const fn color_name(color: capablanca_chess_plus::Color) -> &'static str {
    match color {
        capablanca_chess_plus::Color::White => "white",
        capablanca_chess_plus::Color::Black => "black",
    }
}

const fn result_name(result: CandidateResult) -> &'static str {
    match result {
        CandidateResult::Win => "win",
        CandidateResult::Draw => "draw",
        CandidateResult::Loss => "loss",
    }
}

const fn termination_name(termination: GameTermination) -> &'static str {
    match termination {
        GameTermination::Checkmate => "checkmate",
        GameTermination::Stalemate => "stalemate",
        GameTermination::FiftyMoveRule => "fifty_move",
        GameTermination::ThreefoldRepetition => "threefold",
        GameTermination::ScoreAdjudication => "score_adjudication",
        GameTermination::PlyLimit => "ply_limit",
    }
}

fn print_help() {
    println!(
        "TeraStockfish evaluation comparison\n\
         \n\
         Usage: cargo run --release -p terastockfish --bin terastockfish-eval -- [OPTIONS]\n\
         \n\
         Options:\n\
           --smoke                 One tiny color-swapped pair\n\
           --min-pairs N           Earliest statistical stop (default: 50)\n\
           --max-pairs N           Hard limit (default: 200)\n\
           --nodes N               Node limit per move (default: 100000)\n\
           --max-plies N           Adjudicate longer games as draws (default: 400)\n\
           --opening-plies N       Deterministic opening length (default: 6)\n\
           --hash MB               Hash per player (default: 64)\n\
           --jobs N                Concurrent game pairs (default: 1)\n\
           --adjudication-score N  Sustained winning score; 0 disables\n\
           --adjudication-plies N  Required consecutive half-moves\n\
           --seed N|0xHEX          Opening generator seed\n\
           --set NAME=VALUE        Candidate weight; may be repeated\n\
         \n\
         Weight examples:\n\
           --set material.queen=1700 --set centrality.knight=3\n\
           --set advancement.pawn=8 --set bishop_pair=32 --set tempo=10"
    );
}
