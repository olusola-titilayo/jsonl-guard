use jsonl_guard::{canonical_set, lint, Format, Options, Severity};
use std::process::ExitCode;
use std::{env, fs};

const USAGE: &str = "Usage: jsonl-guard <file.jsonl> [--format chat|completion] [--max-chars N] [--against other.jsonl] [--strict]

Exit codes: 0 clean, 1 problems found, 2 could not run.";

struct Args {
    path: String,
    format: Format,
    strict: bool,
    options: Options,
}

fn parse(mut raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut path = None;
    let mut format = Format::Chat;
    let mut strict = false;
    let mut options = Options::default();
    while let Some(arg) = raw.next() {
        let mut value = |flag: &str| raw.next().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "--format" => {
                format = match value("--format")?.as_str() {
                    "chat" => Format::Chat,
                    "completion" => Format::Completion,
                    other => return Err(format!("unknown format \"{other}\"")),
                }
            }
            "--max-chars" => {
                let text = value("--max-chars")?;
                let limit = text
                    .parse()
                    .map_err(|_| format!("--max-chars: \"{text}\" is not a number"))?;
                options.max_chars = Some(limit);
            }
            "--against" => {
                let other = value("--against")?;
                let text = fs::read_to_string(&other).map_err(|e| format!("{other}: {e}"))?;
                options.other_split = canonical_set(&text);
            }
            "--strict" => strict = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            file if path.is_none() => path = Some(file.to_string()),
            extra => return Err(format!("unexpected argument \"{extra}\"")),
        }
    }
    let path = path.ok_or("no input file given")?;
    Ok(Args {
        path,
        format,
        strict,
        options,
    })
}

fn main() -> ExitCode {
    let args = match parse(env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let input = match fs::read_to_string(&args.path) {
        Ok(input) => input,
        Err(err) => {
            eprintln!("error: {}: {err}", args.path);
            return ExitCode::from(2);
        }
    };

    let report = lint(&input, args.format, &args.options);
    for finding in &report.findings {
        let label = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        println!(
            "{}:{}: {label} [{}] {}",
            args.path, finding.line, finding.rule, finding.message
        );
    }
    println!(
        "{} records, {} error(s), {} warning(s)",
        report.records,
        report.errors(),
        report.warnings()
    );

    if report.errors() > 0 || (args.strict && report.warnings() > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
