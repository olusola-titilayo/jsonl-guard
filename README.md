# jsonl-guard

![CI](https://github.com/olusola-titilayo/jsonl-guard/actions/workflows/ci.yml/badge.svg)

A fast Rust command-line tool that finds problems in an LLM training dataset **before** it reaches a
fine-tuning job. Every finding has a line number, so it can be fixed by hand.

```bash
cargo run -- examples/messy.jsonl
```

```
examples/messy.jsonl:2: error [invalid-json] EOF while parsing a string at line 1 column 51
examples/messy.jsonl:3: error [no-assistant-target] the last message must be from the assistant, or there's nothing to learn
examples/messy.jsonl:4: error [bad-role-order] two user messages in a row
examples/messy.jsonl:5: error [empty-content] message 2 is blank
examples/messy.jsonl:6: warning [duplicate] same content as line 1
5 records, 4 error(s), 1 warning(s)
```

## Rules

| Rule | Severity | Catches |
|---|---|---|
| `invalid-json` | error | A line that doesn't parse. The scan carries on, so one bad line doesn't hide the rest |
| `missing-field`, `empty-content` | error | Absent or whitespace-only text |
| `unknown-role`, `misplaced-system` | error | Roles other than system, user and assistant, or a system message after the start |
| `bad-role-order` | error | A conversation that doesn't start with the user, or repeats a role |
| `no-assistant-target` | error | A conversation with no final assistant reply to learn from |
| `split-leakage` | error | A record also in another split (`--against`), which inflates test scores |
| `duplicate` | warning | Repeated content, even with different key order or spacing around text |
| `too-long` | warning | More characters than `--max-chars`. Counted in characters, not bytes |
| `blank-line` | warning | Blank lines in the file |

## Options

```
jsonl-guard <file.jsonl> [--format chat|completion] [--max-chars N] [--against other.jsonl] [--strict]
```

`--strict` also fails on warnings. Exit codes are `0` clean, `1` problems found and `2` could not run, so it
fits straight into a pipeline.

To check that test data hasn't leaked from training data:

```bash
jsonl-guard test.jsonl --against train.jsonl
```

## Design notes

- **Records are compared in a canonical form.** Key order and spaces around text are ignored, because those
  are the differences that let a "new" record slip through a plain text comparison.
- **It keeps scanning after an error.** Reporting one problem per run makes cleaning a large file very slow.
- **Leakage is an exact-match check.** It won't catch paraphrases, and the tool doesn't claim to.

## Tests

```bash
cargo test
```

Unit tests cover each rule. Integration tests run the compiled binary and check its output and exit codes.
CI also runs `cargo fmt --check` and `cargo clippy -- -D warnings`.

## License

MIT
