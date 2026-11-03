//! Lint LLM datasets stored as JSONL. Every finding points at a line number, so it can be fixed by hand.

use serde_json::Value;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `{"messages": [{"role": "...", "content": "..."}]}`
    Chat,
    /// `{"prompt": "...", "completion": "..."}`
    Completion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    pub severity: Severity,
    pub rule: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Records with more characters than this get a warning.
    pub max_chars: Option<usize>,
    /// Canonical records from another split, used to detect train/test leakage.
    pub other_split: HashSet<String>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub records: usize,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

/// Check a whole dataset. Blank lines are reported but do not count as records.
pub fn lint(input: &str, format: Format, options: &Options) -> Report {
    let mut report = Report::default();
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (index, raw) in input.lines().enumerate() {
        let line = index + 1;
        let mut add = |severity, rule, message: String| {
            report.findings.push(Finding {
                line,
                severity,
                rule,
                message,
            })
        };

        if raw.trim().is_empty() {
            add(
                Severity::Warning,
                "blank-line",
                "blank line in the file".into(),
            );
            continue;
        }
        let value: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(err) => {
                add(Severity::Error, "invalid-json", err.to_string());
                continue;
            }
        };

        let problems = match format {
            Format::Chat => check_chat(&value),
            Format::Completion => check_completion(&value),
        };
        for (severity, rule, message) in problems {
            add(severity, rule, message);
        }

        let key = canonical(&value);
        match seen.entry(key.clone()) {
            Entry::Occupied(first) => add(
                Severity::Warning,
                "duplicate",
                format!("same content as line {}", first.get()),
            ),
            Entry::Vacant(slot) => {
                slot.insert(line);
            }
        }
        if options.other_split.contains(&key) {
            add(
                Severity::Error,
                "split-leakage",
                "this record is also in the other split".into(),
            );
        }
        if let Some(limit) = options.max_chars {
            let length = text_length(&value);
            if length > limit {
                add(
                    Severity::Warning,
                    "too-long",
                    format!("{length} characters, limit is {limit}"),
                );
            }
        }
        report.records += 1;
    }
    report
}

/// A form of the record that ignores key order and spacing around text, used to compare records.
pub fn canonical(value: &Value) -> String {
    fn normalise(value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(text.trim().to_string()),
            Value::Array(items) => Value::Array(items.iter().map(normalise).collect()),
            // serde_json's default map keeps keys sorted, so key order in the source doesn't matter.
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, item)| (key.clone(), normalise(item)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    normalise(value).to_string()
}

/// Canonical forms of every parseable line, for use as `Options::other_split`.
pub fn canonical_set(input: &str) -> HashSet<String> {
    input
        .lines()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .map(|value| canonical(&value))
        .collect()
}

type Problem = (Severity, &'static str, String);

fn check_chat(value: &Value) -> Vec<Problem> {
    let mut problems = Vec::new();
    let Some(messages) = value.get("messages").and_then(Value::as_array) else {
        problems.push(error("missing-field", "needs a \"messages\" array"));
        return problems;
    };
    if messages.is_empty() {
        problems.push(error("empty-conversation", "\"messages\" is empty"));
        return problems;
    }

    let mut roles = Vec::with_capacity(messages.len());
    for (position, message) in messages.iter().enumerate() {
        let number = position + 1;
        match message.get("role").and_then(Value::as_str) {
            Some(role @ ("system" | "user" | "assistant")) => roles.push(role),
            Some(role) => {
                problems.push(error(
                    "unknown-role",
                    &format!("message {number}: \"{role}\""),
                ));
                return problems;
            }
            None => {
                problems.push(error(
                    "missing-field",
                    &format!("message {number} has no role"),
                ));
                return problems;
            }
        }
        match message.get("content").and_then(Value::as_str) {
            Some(text) if text.trim().is_empty() => {
                problems.push(error(
                    "empty-content",
                    &format!("message {number} is blank"),
                ));
            }
            Some(_) => {}
            None => problems.push(error(
                "missing-field",
                &format!("message {number} has no text content"),
            )),
        }
    }

    if roles[1..].contains(&"system") {
        problems.push(error(
            "misplaced-system",
            "a system message can only come first",
        ));
    }
    let turns: Vec<&str> = roles.iter().copied().filter(|r| *r != "system").collect();
    if turns.first() != Some(&"user") {
        problems.push(error(
            "bad-role-order",
            "the conversation must start with a user message",
        ));
    }
    if let Some(pair) = turns.windows(2).find(|pair| pair[0] == pair[1]) {
        problems.push(error(
            "bad-role-order",
            &format!("two {} messages in a row", pair[0]),
        ));
    }
    if turns.last() != Some(&"assistant") {
        problems.push(error(
            "no-assistant-target",
            "the last message must be from the assistant, or there's nothing to learn",
        ));
    }
    problems
}

fn check_completion(value: &Value) -> Vec<Problem> {
    let mut problems = Vec::new();
    for field in ["prompt", "completion"] {
        match value.get(field).and_then(Value::as_str) {
            Some(text) if text.trim().is_empty() => {
                problems.push(error("empty-content", &format!("\"{field}\" is blank")));
            }
            Some(_) => {}
            None => problems.push(error("missing-field", &format!("needs a text \"{field}\""))),
        }
    }
    problems
}

fn error(rule: &'static str, message: &str) -> Problem {
    (Severity::Error, rule, message.to_string())
}

fn text_length(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(text_length).sum(),
        Value::Object(map) => map.values().map(text_length).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(input: &str, format: Format) -> Vec<&'static str> {
        lint(input, format, &Options::default())
            .findings
            .iter()
            .map(|f| f.rule)
            .collect()
    }

    fn chat(roles: &[&str]) -> String {
        let messages: Vec<Value> = roles
            .iter()
            .enumerate()
            .map(|(i, role)| serde_json::json!({"role": role, "content": format!("text {i}")}))
            .collect();
        serde_json::json!({ "messages": messages }).to_string()
    }

    #[test]
    fn valid_conversations_have_no_findings() {
        assert!(rules(&chat(&["user", "assistant"]), Format::Chat).is_empty());
        assert!(rules(&chat(&["system", "user", "assistant"]), Format::Chat).is_empty());
        assert!(rules(
            &chat(&["user", "assistant", "user", "assistant"]),
            Format::Chat
        )
        .is_empty());
    }

    #[test]
    fn role_order_problems_are_caught() {
        assert_eq!(
            rules(&chat(&["assistant", "user", "assistant"]), Format::Chat),
            ["bad-role-order"]
        );
        assert_eq!(
            rules(&chat(&["user", "user", "assistant"]), Format::Chat),
            ["bad-role-order"]
        );
        assert_eq!(
            rules(&chat(&["user", "assistant", "user"]), Format::Chat),
            ["no-assistant-target"]
        );
        assert_eq!(
            rules(&chat(&["user", "system", "assistant"]), Format::Chat),
            ["misplaced-system"]
        );
    }

    #[test]
    fn unknown_role_stops_further_order_checks() {
        assert_eq!(
            rules(&chat(&["user", "tool"]), Format::Chat),
            ["unknown-role"]
        );
    }

    #[test]
    fn blank_and_missing_content_is_an_error() {
        let blank =
            r#"{"messages":[{"role":"user","content":"  "},{"role":"assistant","content":"x"}]}"#;
        assert_eq!(rules(blank, Format::Chat), ["empty-content"]);
        let missing = r#"{"messages":[{"role":"user"},{"role":"assistant","content":"x"}]}"#;
        assert_eq!(rules(missing, Format::Chat), ["missing-field"]);
    }

    #[test]
    fn completion_format_needs_both_fields() {
        assert!(rules(r#"{"prompt":"a","completion":"b"}"#, Format::Completion).is_empty());
        assert_eq!(
            rules(r#"{"prompt":"a"}"#, Format::Completion),
            ["missing-field"]
        );
        assert_eq!(
            rules(r#"{"prompt":"a","completion":" "}"#, Format::Completion),
            ["empty-content"]
        );
    }

    #[test]
    fn duplicates_ignore_key_order_and_surrounding_spaces() {
        let input =
            "{\"prompt\":\"a\",\"completion\":\"b\"}\n{\"completion\":\" b \",\"prompt\":\"a\"}";
        let report = lint(input, Format::Completion, &Options::default());
        assert_eq!(report.findings[0].rule, "duplicate");
        assert_eq!(report.findings[0].line, 2);
        assert_eq!(report.findings[0].message, "same content as line 1");
    }

    #[test]
    fn leakage_is_found_across_splits() {
        let train = "{\"prompt\":\"a\",\"completion\":\"b\"}\n";
        let test =
            "{\"completion\":\"b\",\"prompt\":\"a\"}\n{\"prompt\":\"c\",\"completion\":\"d\"}\n";
        let options = Options {
            other_split: canonical_set(train),
            ..Options::default()
        };
        let report = lint(test, Format::Completion, &options);
        assert_eq!(report.errors(), 1);
        assert_eq!(report.findings[0].rule, "split-leakage");
        assert_eq!(report.findings[0].line, 1);
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        let options = Options {
            max_chars: Some(3),
            ..Options::default()
        };
        // "éé" is 2 characters but 4 bytes, so it must stay under the limit of 3.
        let short = r#"{"prompt":"é","completion":"é"}"#;
        assert_eq!(lint(short, Format::Completion, &options).warnings(), 0);
        let long = r#"{"prompt":"ab","completion":"cd"}"#;
        assert_eq!(lint(long, Format::Completion, &options).warnings(), 1);
    }

    #[test]
    fn broken_lines_do_not_stop_the_scan_or_count_as_records() {
        let input = "not json\n\n{\"prompt\":\"a\",\"completion\":\"b\"}\n";
        let report = lint(input, Format::Completion, &Options::default());
        assert_eq!(report.records, 1);
        assert_eq!(report.errors(), 1);
        assert_eq!(report.warnings(), 1);
        assert_eq!(report.findings[0].line, 1);
        assert_eq!(report.findings[1].line, 2);
    }
}
