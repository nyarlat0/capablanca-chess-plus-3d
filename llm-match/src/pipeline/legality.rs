//! Decode only the proposed move and validate it on a clone. No move suggestions.
use super::*;
pub(super) fn move_text(value: &Value, config: &Config) -> Result<String> {
    if config.classic.is_none() && config.representation.output_format == OutputFormat::NumericJson
    {
        ensure!(value.is_object(), "move must be a JSON object");
        Ok(serde_json::to_string(value)?)
    } else {
        let text = value.as_str().context("move must be a string")?;
        bounded(text, 160)?;
        ensure!(
            text == text.trim(),
            "move must not have surrounding whitespace"
        );
        Ok(text.into())
    }
}
pub(super) fn check_move(value: &Value, state: &Match, config: &Config) -> Result<String> {
    let text = move_text(value, config)?;
    if config.classic.is_some() {
        ensure!(classic::san_shape(&text), "expected canonical SAN syntax");
    } else {
        crate::parse_model_move(
            &text,
            config.representation.output_format,
            state.game.position(),
        )?;
    }
    Ok(text)
}
pub(super) fn square(value: &Value, state: &Match, config: &Config) -> Result<Square> {
    if config.classic.is_none() && config.representation.state_format != StateFormat::PiecesAscii {
        crate::square_from_numeric(
            serde_json::from_value(value.clone())?,
            state.game.position().board().size(),
        )
    } else {
        let text = value.as_str().context("expected square string")?;
        let square: Square = text.parse()?;
        ensure!(square.to_string() == text, "noncanonical square");
        let size = state.game.position().board().size();
        ensure!(
            square.file() < size.files() && square.rank() < size.ranks(),
            "square outside board"
        );
        Ok(square)
    }
}

pub(super) fn validate_candidate(
    state: &Match,
    config: &Config,
    candidate: &Candidate,
) -> Result<(Match, Move)> {
    let (trial, mv) = resolve_candidate(state, config, candidate)?;
    validate_metadata(state, config, candidate, mv)?;
    Ok((trial, mv))
}

pub(super) fn resolve_candidate(
    state: &Match,
    config: &Config,
    candidate: &Candidate,
) -> Result<(Match, Move)> {
    let mut trial = state.clone();
    check_move(&candidate.proposed_move, state, config)?;
    let text = move_text(&candidate.proposed_move, config)?;
    let mv = trial.accept_configured_answer(&text, config)?;
    Ok((trial, mv))
}

pub(super) fn validate_metadata(
    state: &Match,
    config: &Config,
    candidate: &Candidate,
    mv: Move,
) -> Result<()> {
    ensure!(
        mv.from == square(&candidate.from, state, config)?
            && mv.to == square(&candidate.to, state, config)?,
        "candidate coordinates differ from move"
    );
    Ok(())
}
