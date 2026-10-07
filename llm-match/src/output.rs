use anyhow::{Result, ensure};
use prompt_core::ReasoningTemplate;

/// None means an incomplete/empty reasoning generation, never model feedback.
/// Removes only explicit reasoning blocks, never extracts a move from prose.
pub(crate) fn final_answer(template: &ReasoningTemplate, raw: &str) -> Result<Option<String>> {
    ensure!(
        template.prefix.is_empty() == template.suffix.is_empty(),
        "reasoning prefix and suffix must both be configured (or both empty)"
    );
    if template.prefix.is_empty() {
        return Ok(Some(raw.trim().to_owned()));
    }
    let mut markers = vec![template.prefix.clone()];
    // Artemis includes transport channel tokens before <thinking>. The model
    // may emit just the XML block explicitly requested by the chess template.
    if let Some(tag) = template
        .suffix
        .strip_prefix("</")
        .and_then(|s| s.strip_suffix('>'))
    {
        let opening = format!("<{tag}>");
        if template.prefix.ends_with(&opening) {
            markers.push(opening.clone());
            let channel = template.prefix.trim_end_matches(&opening);
            if !channel.is_empty() {
                // A generation cut off before the XML opener is also reasoning.
                markers.push(channel.to_owned());
            }
        }
    }
    let mut result = String::new();
    let mut rest = raw;
    let mut had_reasoning = false;
    loop {
        let next = markers
            .iter()
            .filter_map(|marker| rest.find(marker).map(|i| (i, marker)))
            .min_by_key(|(i, marker)| (*i, std::cmp::Reverse(marker.len())));
        let Some((start, prefix)) = next else {
            result.push_str(rest);
            break;
        };
        had_reasoning = true;
        // A reasoning block must precede the final answer, not splice together
        // coordinate fragments or extract a move preceding reasoning.
        if !rest[..start].trim().is_empty() {
            return Ok(None);
        }
        let body = &rest[start + prefix.len()..];
        let Some(end) = body.find(&template.suffix) else {
            return Ok(None);
        };
        rest = &body[end + template.suffix.len()..];
        if !template.separator.is_empty() {
            rest = rest.strip_prefix(&template.separator).unwrap_or(rest);
        }
    }
    let answer = result.trim().to_owned();
    if had_reasoning && answer.is_empty() {
        Ok(None)
    } else {
        Ok(Some(answer))
    }
}

/// Syntax gate, not legality. Bad prose/empty output is never repeated to the
/// model. Board bounds, piece identities and promotions are engine decisions.
pub(crate) fn is_coordinate_move(text: &str) -> bool {
    let mut bytes = text.as_bytes();
    for _ in 0..2 {
        if !bytes.first().is_some_and(u8::is_ascii_lowercase) {
            return false;
        }
        bytes = &bytes[1..];
        if !bytes.first().is_some_and(|b| matches!(b, b'1'..=b'9')) {
            return false;
        }
        let count = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
        if count > 3 {
            return false;
        }
        bytes = &bytes[count..];
    }
    bytes.is_empty() || (bytes.len() == 1 && bytes[0].is_ascii_lowercase())
}

pub(crate) fn transient(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|e| e.downcast_ref::<reqwest::Error>())
        .any(|e| {
            e.is_timeout()
                || e.is_connect()
                || e.status()
                    .is_some_and(|s| s.as_u16() == 408 || s.as_u16() == 429 || s.is_server_error())
        })
}
