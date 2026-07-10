//! Structured attention reports for autonomous check-ins.
//!
//! Heartbeat and routine runs both end with the same question: does
//! anything need the user's attention? This used to be answered by
//! sentinel-string matching (`content.contains("HEARTBEAT_OK")`), which
//! breaks whenever the model mentions the sentinel while ALSO reporting a
//! finding. The model now returns a small JSON object instead; parsing it
//! is structural, not semantic.

use serde::Deserialize;

/// Parsed outcome of an autonomous check-in run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionReport {
    pub needs_attention: bool,
    /// Present when `needs_attention` is true (or when parsing fell back
    /// to the raw content).
    pub summary: Option<String>,
}

/// Instructions appended to heartbeat/routine prompts.
///
/// Kept as one shared constant so both callers ask for the identical
/// shape and `parse_attention_report` has a single contract to honor.
pub const ATTENTION_FORMAT_INSTRUCTIONS: &str = "\
Respond with a single JSON object and nothing else:\n\
{\"needs_attention\": <true|false>, \"summary\": \"<if true: a concise summary of what needs action; if false: empty string>\"}";

#[derive(Deserialize)]
struct WireReport {
    needs_attention: bool,
    #[serde(default)]
    summary: String,
}

/// Parse a model check-in response into an [`AttentionReport`].
///
/// Accepts the bare JSON object, one wrapped in a Markdown code fence,
/// or a JSON object embedded in surrounding prose (chat-tuned models
/// often add a sentence around the requested JSON — extracting the
/// `{...}` substring is structural, and absorbs the most common
/// formatting deviation instead of paging the user about it).
/// On any parse failure the report FAILS OPEN to `needs_attention: true`
/// with the raw content as the summary — a spurious notification is
/// recoverable, a silently dropped alert is not.
pub fn parse_attention_report(content: &str) -> AttentionReport {
    let trimmed = content.trim();

    let candidate = strip_code_fence(trimmed);

    let wire = serde_json::from_str::<WireReport>(candidate)
        .ok()
        .or_else(|| {
            // Second pass: the outermost {...} span within prose.
            let start = candidate.find('{')?;
            let end = candidate.rfind('}')?;
            serde_json::from_str::<WireReport>(candidate.get(start..=end)?).ok()
        });

    match wire {
        Some(wire) => {
            let summary = wire.summary.trim();
            AttentionReport {
                needs_attention: wire.needs_attention,
                summary: if summary.is_empty() {
                    None
                } else {
                    Some(summary.to_string())
                },
            }
        }
        None => AttentionReport {
            needs_attention: true,
            summary: Some(trimmed.to_string()),
        },
    }
}

/// Strip a surrounding Markdown code fence (``` or ```json) if present.
/// Structural unwrapping only — no content inspection.
fn strip_code_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    // Drop an optional language tag on the fence line.
    let rest = match rest.split_once('\n') {
        Some((_lang, body)) => body,
        None => rest,
    };
    rest.strip_suffix("```").map(str::trim).unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ok_report() {
        let r = parse_attention_report(r#"{"needs_attention": false, "summary": ""}"#);
        assert!(!r.needs_attention);
        assert!(r.summary.is_none());
    }

    #[test]
    fn parses_attention_report_with_summary() {
        let r = parse_attention_report(
            r#"{"needs_attention": true, "summary": "Disk usage at 95% on /"}"#,
        );
        assert!(r.needs_attention);
        assert_eq!(r.summary.as_deref(), Some("Disk usage at 95% on /"));
    }

    #[test]
    fn parses_fenced_json() {
        let r = parse_attention_report(
            "```json\n{\"needs_attention\": false, \"summary\": \"\"}\n```",
        );
        assert!(!r.needs_attention);
    }

    // The failure mode the sentinel approach couldn't handle: a report
    // that MENTIONS the ok-marker while carrying a real finding. With
    // structured output the flag is unambiguous.
    #[test]
    fn attention_true_wins_even_with_ok_language() {
        let r = parse_attention_report(
            r#"{"needs_attention": true, "summary": "All checks OK except backup job failed"}"#,
        );
        assert!(r.needs_attention);
    }

    // Non-JSON output fails OPEN: notify with the raw content.
    #[test]
    fn unparseable_fails_open_to_attention() {
        let r = parse_attention_report("Everything looks fine today!");
        assert!(r.needs_attention);
        assert_eq!(r.summary.as_deref(), Some("Everything looks fine today!"));
    }

    // JSON embedded in prose is extracted — a quiet routine whose model
    // adds a sentence around the JSON must not page the user.
    #[test]
    fn json_embedded_in_prose_is_extracted() {
        let r = parse_attention_report(
            "Here is my check-in: {\"needs_attention\": false, \"summary\": \"\"} \
             Let me know if you need anything else!",
        );
        assert!(!r.needs_attention);
    }

    #[test]
    fn missing_summary_field_defaults_empty() {
        let r = parse_attention_report(r#"{"needs_attention": false}"#);
        assert!(!r.needs_attention);
        assert!(r.summary.is_none());
    }
}
