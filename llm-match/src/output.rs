use anyhow::{Result, ensure};
use prompt_core::ReasoningTemplate;

/// Parse only the final object (or final JSON fence), ignoring preceding prose.
/// Locate the boundary before typed deserialization: an invalid final schema
/// must never cause fallback to an earlier object or a nested object.
pub(crate) fn parse_final_json<T: serde::de::DeserializeOwned>(answer: &str) -> Result<T> {
    let answer = answer.trim();
    if let Some((before, closing)) = answer.rsplit_once('\n')
        && closing.trim() == "```"
    {
        for (offset, _) in before.rmatch_indices("```") {
            let line_start = before[..offset].rfind('\n').map_or(0, |i| i + 1);
            if !before[line_start..offset].trim().is_empty() {
                continue;
            }
            let fence = &answer[offset..];
            let header = fence.lines().next().unwrap_or_default().trim_end();
            ensure!(
                matches!(header, "```json" | "```"),
                "expected a final JSON fence"
            );
            return parse_json_repaired(fence);
        }
        anyhow::bail!("missing opening JSON fence");
    }
    ensure!(answer.ends_with('}'), "expected a final JSON object");
    let bytes = answer.as_bytes();
    let mut stack = Vec::new();
    let mut quoted = false;
    for i in (0..bytes.len()).rev() {
        let byte = bytes[i];
        if byte == b'"' {
            let escapes = bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count();
            if escapes % 2 == 0 {
                quoted = !quoted;
            }
        }
        if quoted || byte == b'"' {
            continue;
        }
        match byte {
            b'}' | b']' => stack.push(byte),
            b'{' | b'[' => {
                let expected = if byte == b'{' { b'}' } else { b']' };
                ensure!(stack.pop() == Some(expected), "mismatched JSON delimiters");
                if stack.is_empty() {
                    return parse_json_repaired(&answer[i..]);
                }
            }
            _ => {}
        }
    }
    anyhow::bail!("incomplete final JSON object")
}

/// Syntax-only local recovery: fences and trailing commas outside strings.
/// No content extraction, field defaults, move substitutions or legality repair.
pub(crate) fn parse_json_repaired<T: serde::de::DeserializeOwned>(answer: &str) -> Result<T> {
    if let Ok(value) = parse_json(answer) {
        return Ok(value);
    }
    let trimmed = answer.trim();
    let body = if trimmed.starts_with("```") {
        let (header, rest) = trimmed
            .split_once('\n')
            .ok_or_else(|| anyhow::anyhow!("missing fenced body"))?;
        ensure!(
            matches!(header.trim_end(), "```json" | "```"),
            "not a JSON fence"
        );
        match rest.rsplit_once('\n') {
            Some((body, closing)) if closing.trim() == "```" => body,
            _ => rest, // A missing closing Markdown fence does not truncate a complete JSON value.
        }
    } else {
        trimmed
    };
    let bytes = body.as_bytes();
    let mut fixed = Vec::with_capacity(bytes.len());
    let (mut string, mut escaped) = (false, false);
    for (i, &byte) in bytes.iter().enumerate() {
        if string {
            fixed.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
        } else {
            if byte == b'"' {
                string = true;
            }
            if byte == b','
                && bytes[i + 1..]
                    .iter()
                    .find(|b| !b.is_ascii_whitespace())
                    .is_some_and(|b| matches!(b, b'}' | b']'))
            {
                continue;
            }
            fixed.push(byte);
        }
    }
    let value = serde_json::from_slice(&fixed)?;
    tracing::info!(reason = "json_syntax_repaired_locally", "llm_format_repair");
    Ok(value)
}

/// Accept a whole JSON response, optionally wrapped in one Markdown code block.
/// Never extract objects from prose or choose between multiple blocks.
pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(answer: &str) -> Result<T> {
    let answer = answer.trim();
    let payload = if answer.starts_with("```") {
        let (header, rest) = answer
            .split_once('\n')
            .ok_or_else(|| anyhow::anyhow!("unclosed JSON code block"))?;
        ensure!(
            matches!(header.trim_end(), "```json" | "```"),
            "expected a JSON code block"
        );
        let (body, closing) = rest
            .rsplit_once('\n')
            .ok_or_else(|| anyhow::anyhow!("missing closing code fence"))?;
        ensure!(closing.trim() == "```", "missing closing code fence");
        body
    } else {
        answer
    };
    Ok(serde_json::from_str(payload)?)
}

#[cfg(test)]
mod json_fence_tests {
    use super::*;

    #[test]
    fn final_json_ignores_prefix_but_keeps_the_final_schema_strict() {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Choice {
            choice: String,
        }
        for raw in [
            "Рассуждения { arbitrary text\n{\"choice\":\"c2\"}",
            "{\"choice\":\"c1\"}\nThen:\n```json\n{\"choice\":\"c2\",}\n```",
            "Reasoning\n```\r\n{\"choice\":\"c2\"}\r\n```  ",
        ] {
            assert_eq!(parse_final_json::<Choice>(raw).unwrap().choice, "c2");
        }
        for raw in [
            "{\"choice\":\"c1\"}\n{\"wrong\":{\"choice\":\"c2\"}}",
            "{\"choice\":\"c1\"}\n{\"choice\":}",
            "{\"choice\":\"c1\",\"choice\":\"c2\"}",
            "{\"choice\":\"c1\"} trailing prose",
            "Earlier\n```python\n{\"choice\":\"c1\"}\n```",
        ] {
            assert!(parse_final_json::<Choice>(raw).is_err(), "{raw}");
        }
        let value: serde_json::Value = parse_final_json(
            r#"Prefix " { fake } {"move":{"from":[2,8],"to":[3,6]},"rationale":"brace } and escaped \" and ```"}"#
        ).unwrap();
        assert_eq!(value["move"]["from"][0], 2);
    }

    #[test]
    fn local_repair_never_changes_strings_or_infers_missing_content() {
        let value: serde_json::Value =
            parse_json_repaired(r#"{"s":"literal ,} and escaped \"",}"#).unwrap();
        assert_eq!(value["s"], "literal ,} and escaped \"");
        for raw in [
            r#"{"s":"unfinished"#,
            r#"{"s":}"#,
            r#"prose {"s":1}"#,
            "```json\n{}\n```\n```json\n{}\n```",
            "{'choice':'c1'}",
        ] {
            assert!(parse_json_repaired::<serde_json::Value>(raw).is_err());
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Only {
            #[allow(dead_code)]
            choice: String,
        }
        for raw in [
            r#"{"choice":"c1","choice":"c2",}"#,
            r#"{"choice":"c1","extra":true,}"#,
        ] {
            assert!(parse_json_repaired::<Only>(raw).is_err());
        }
    }

    #[test]
    fn accepts_plain_and_fenced_json() {
        for text in [
            r#"{"move":"e4"}"#,
            "```json\n{\"move\":\"e4\"}\n```",
            "  ```\r\n{\"move\":\"e4\"}\r\n```\n",
        ] {
            let value: serde_json::Value = parse_json(text).unwrap();
            assert_eq!(value["move"], "e4");
        }
    }

    #[test]
    fn rejects_prose_multiple_blocks_and_invalid_json() {
        for text in [
            "Here:\n```json\n{}\n```",
            "```json\n{}\n```\nThanks",
            "```json\n{}",
            "```json\n{}\n```\n```json\n{}\n```",
            "```python\n{}\n```",
            "```json\n{bad}\n```",
            "```json\n{} {}\n```",
        ] {
            assert!(parse_json::<serde_json::Value>(text).is_err(), "{text}");
        }
    }
}

/// A complete move must end the answer. Never choose an earlier legal move
/// instead of the final attempt; the predicate checks representation only.
pub(crate) fn trailing_move(text: &str, syntax: impl Fn(&str) -> bool) -> Option<&str> {
    let text = text.trim();
    std::iter::once(0)
        .chain(text.char_indices().filter_map(|(i, c)| {
            (c.is_whitespace() || matches!(c, ':' | '>' | '`' | '"' | '.' | ',' | ';' | '(' | '['))
                .then_some(i + c.len_utf8())
        }))
        .map(|i| &text[i..])
        .find(|s| !s.is_empty() && syntax(s))
}

/// None means an incomplete/empty reasoning generation, never model feedback.
/// Removes only explicit reasoning blocks, never extracts a move from prose.
pub(crate) fn final_answer(template: &ReasoningTemplate, raw: &str) -> Result<Option<String>> {
    final_answer_impl(template, raw, false)
}

/// Structured stages allow prose before reasoning, but still discard the entire
/// generation when any reasoning block is unfinished or has no final answer.
pub(crate) fn final_json_answer(template: &ReasoningTemplate, raw: &str) -> Result<Option<String>> {
    final_answer_impl(template, raw, true)
}

fn final_answer_impl(
    template: &ReasoningTemplate,
    raw: &str,
    allow_prose: bool,
) -> Result<Option<String>> {
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
        if !allow_prose && !rest[..start].trim().is_empty() {
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
