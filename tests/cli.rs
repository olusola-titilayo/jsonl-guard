use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jsonl-guard"))
        .args(args)
        .output()
        .expect("binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn clean_file_exits_zero() {
    let output = run(&["examples/clean.jsonl"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stdout(&output).contains("2 records, 0 error(s), 0 warning(s)"));
}

#[test]
fn messy_file_reports_every_problem_with_its_line_and_exits_one() {
    let output = run(&["examples/messy.jsonl"]);
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(1));
    for expected in [
        "messy.jsonl:2: error [invalid-json]",
        "messy.jsonl:3: error [no-assistant-target]",
        "messy.jsonl:4: error [bad-role-order]",
        "messy.jsonl:5: error [empty-content]",
        "messy.jsonl:6: warning [duplicate] same content as line 1",
    ] {
        assert!(text.contains(expected), "missing {expected} in:\n{text}");
    }
}

#[test]
fn leakage_is_reported_against_another_split() {
    let output = run(&["examples/clean.jsonl", "--against", "examples/messy.jsonl"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("[split-leakage]"));
}

#[test]
fn strict_turns_warnings_into_failure() {
    let lenient = run(&["examples/clean.jsonl", "--max-chars", "5"]);
    assert_eq!(lenient.status.code(), Some(0));
    let strict = run(&["examples/clean.jsonl", "--max-chars", "5", "--strict"]);
    assert_eq!(strict.status.code(), Some(1));
}

#[test]
fn usage_errors_exit_two() {
    assert_eq!(run(&[]).status.code(), Some(2));
    assert_eq!(run(&["nope.jsonl"]).status.code(), Some(2));
    assert_eq!(
        run(&["examples/clean.jsonl", "--format", "xml"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        run(&["examples/clean.jsonl", "--max-chars", "lots"])
            .status
            .code(),
        Some(2)
    );
}
