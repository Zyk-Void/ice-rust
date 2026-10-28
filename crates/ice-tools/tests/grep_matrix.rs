//! Differential matrix for the `grep` tool. Generated cases run through the
//! ripgrep engine and the in-process engine (both on the same files on disk),
//! and through ugrep (`archive`) and ast-grep (`ast`) when installed. Each
//! result is checked against the raw engine CLI as the oracle:
//!
//! - with no limit pressure, the match and context lines equal the oracle's;
//! - under the match limit or byte cap, results are a subset of the oracle's,
//!   and the limit notice is present;
//! - errors (bad regex, missing path, invalid mode combinations) agree.
//!
//! Known, intended differences are normalised, not hidden: the in-process
//! engine repeats overlapping context lines (as ice does, so a match line may
//! also appear as context), and reports the limit when the last match hits
//! it exactly. Set `GREP_MATRIX_SEED` / `GREP_MATRIX_CASES` to explore.
//!
//! Unix only: the fixture uses symlinks and `:` in names, and rg prints `\`
//! separators on Windows, which this oracle does not model.
#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use ice_agent::agent_tool::AgentTool;
use ice_agent::types::TextContentOrImage;
use ice_tools::{ExecutionToolContext, OsExecutionEnv, GREP_MAX_LINE_LENGTH};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

/// xorshift64*: deterministic cases without a `rand` dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())].clone()
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn write(root: &Path, rel: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A small repository exercising ignore files, hidden and binary files, odd
/// names, CRLF, long lines, unicode and (for `archive` mode) archives.
fn build_fixture(root: &Path, archives: bool) {
    std::fs::create_dir_all(root.join(".git")).unwrap();
    write(root, ".gitignore", "ignored/\n*.log\n/build\n");
    write(root, ".ignore", "skip_me.txt\n");
    write(root, ".rgignore", "rgskip.txt\n");
    write(
        root,
        "top.txt",
        "needle at top\nTop Needle mixed case\nplain line\n",
    );
    write(root, "skip_me.txt", "needle hidden by .ignore\n");
    write(root, "rgskip.txt", "needle hidden by .rgignore\n");
    write(root, "debug.log", "needle in log\n");
    write(root, "ignored/x.txt", "needle in ignored dir\n");
    write(root, "build/out.txt", "needle in build\n");
    write(root, ".hidden/secret.txt", "needle in hidden dir\n");
    write(root, ".env.sample", "NEEDLE=1\nfoo=bar\n");
    write(
        root,
        "src/lib.rs",
        "// needle: library docs\n\
         pub fn alpha(x: u32) -> u32 {\n    x + 1\n}\n\n\
         pub struct Foo;\npub struct Bar;\n\n\
         impl Clone for Foo {\n    fn clone(&self) -> Self {\n        Foo\n    }\n}\n\n\
         fn beta() {\n    let value = compute().unwrap();\n    log(\"12:30:45 needle tick\");\n}\n\n\
         // a.b(c) literal marker\n\
         fn gamma() { foo(1); foo(2); }\n",
    );
    write(
        root,
        "src/main.rs",
        "fn main() {\n    let needle = 1;\n    let other = needle + 1;\n    println!(\"{}\", other);\n    foo(needle);\n}\n",
    );
    write(root, "src/.gitignore", "gen.rs\n!keep.log\n");
    write(root, "src/gen.rs", "// needle generated\n");
    write(root, "src/keep.log", "needle kept log\n");
    write(
        root,
        "src/nested/deep/mod.rs",
        "pub mod deep {\n    // needle deep\n    pub fn delta() -> Option<u8> { None }\n}\n",
    );
    write(
        root,
        "src/nested/util.ts",
        "export function util(a: number) {\n  console.log(\"needle\", a);\n  return a;\n}\nconst x = 1;\nconsole.log(x);\n",
    );
    write(
        root,
        "docs/guide.md",
        "# Guide\n\nThe needle section.\n日本語 needle\ncafé needle\nNEEDLE upper\n\n- item\n",
    );
    let notes: String = (1..=300)
        .map(|n| {
            if n % 7 == 0 {
                format!("line {n} needle {n}\n")
            } else {
                format!("line {n} filler\n")
            }
        })
        .collect();
    write(root, "docs/notes.txt", notes);
    write(root, "crlf.txt", "needle one;\r\nsecond;\r\nneedle two\r\n");
    write(root, "noeol.txt", "first\nneedle without eol");
    write(root, "empty.txt", "");
    write(
        root,
        "long.txt",
        format!(
            "{} needle {}\nshort needle\n",
            "x".repeat(1200),
            "y".repeat(40)
        ),
    );
    write(root, "a-1-b.txt", "needle in dashed name\n");
    write(root, "space name.txt", "needle in spaced name\n");
    write(root, "bin.dat", b"ab\0cd needle\n");
    let mut late = b"needle before a late nul\n".to_vec();
    late.extend(std::iter::repeat_n(b'z', 10_000));
    late.extend(b"\0\n");
    write(root, "late_nul.txt", late);
    write(root, "colon:2:name.txt", "needle in colon name\n");
    std::os::unix::fs::symlink(root.join("top.txt"), root.join("link.txt")).unwrap();
    if !archives {
        return;
    }
    // Archives for `archive` mode (binary to the text engines).
    write(
        root,
        "pack/inner/needle.txt",
        "needle inside archive\nplain\n",
    );
    let tar = Command::new("tar")
        .args(["cf", "archive.tar", "-C", "pack", "inner/needle.txt"])
        .current_dir(root)
        .status();
    assert!(tar.is_ok_and(|s| s.success()), "tar is required");
    std::fs::remove_dir_all(root.join("pack")).unwrap();
    write(root, "zipped.txt", "needle in gzip\nplain gz\n");
    let gzip = Command::new("gzip")
        .arg("zipped.txt")
        .current_dir(root)
        .status();
    assert!(gzip.is_ok_and(|s| s.success()), "gzip is required");
}

/// Every display path the fixture can produce, longest first, so output
/// lines are split on a known path instead of guessing at separators.
fn known_paths(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in std::fs::read_dir(root.join(&rel)).unwrap() {
            let entry = entry.unwrap();
            let rel = rel.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                stack.push(rel);
            } else {
                paths.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    paths.push("archive.tar{inner/needle.txt}".into());
    paths.push("zipped.txt.gz{zipped.txt}".into());
    paths.sort_by_key(|path| std::cmp::Reverse(path.len()));
    paths
}

type Key = (String, u64);

/// Line texts by `(path, line)`; ast-grep can report several matches on one
/// line, so each key holds a set.
#[derive(Debug, Default)]
struct Records {
    matches: BTreeMap<Key, BTreeSet<String>>,
    context: BTreeMap<Key, BTreeSet<String>>,
}

impl Records {
    fn add(&mut self, is_match: bool, key: Key, text: String) {
        let map = if is_match {
            &mut self.matches
        } else {
            &mut self.context
        };
        map.entry(key).or_default().insert(text);
    }
    fn match_count(&self) -> usize {
        self.matches.values().map(BTreeSet::len).sum()
    }
}

#[derive(Debug)]
struct Parsed {
    records: Records,
    notices: String,
    unparsed: Vec<String>,
    no_matches: bool,
}

fn parse_tool(text: &str, paths: &[String]) -> Parsed {
    let (body, notices) = match text.find("\n\n[") {
        Some(index) => (&text[..index], &text[index..]),
        None => (text, ""),
    };
    let mut parsed = Parsed {
        records: Records::default(),
        notices: notices.to_owned(),
        unparsed: Vec::new(),
        no_matches: body == "No matches found",
    };
    if parsed.no_matches {
        return parsed;
    }
    for line in body.lines() {
        if line == "--" || line.is_empty() {
            continue;
        }
        let record = paths.iter().find_map(|path| {
            let rest = line.strip_prefix(path.as_str())?;
            let sep = rest.chars().next().filter(|c| *c == ':' || *c == '-')?;
            let rest = &rest[1..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let tail = rest[digits..].strip_prefix(sep)?.strip_prefix(' ')?;
            Some((
                sep == ':',
                (path.clone(), rest[..digits].parse().ok()?),
                tail,
            ))
        });
        match record {
            Some((is_match, key, text)) => parsed.records.add(is_match, key, text.to_owned()),
            None => parsed.unparsed.push(line.to_owned()),
        }
    }
    parsed
}

/// The line text the tool promises: CR removed, at most 500 characters.
fn expected_text(raw: &str) -> String {
    let line = raw.strip_suffix('\n').unwrap_or(raw);
    let line = line.replace('\r', "");
    if line.chars().count() <= GREP_MAX_LINE_LENGTH {
        line
    } else {
        let head: String = line.chars().take(GREP_MAX_LINE_LENGTH).collect();
        format!("{head}... [truncated]")
    }
}

fn normalize_path(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).replace('\\', "/")
}

struct Oracle {
    records: Records,
    error: bool,
    /// Engine lines the oracle could not classify (reported as failures).
    unparsed: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Text,
    Archive,
    Ast,
}

#[derive(Clone, Debug)]
struct Case {
    mode: Mode,
    params: Value,
}

impl Case {
    fn paths(&self) -> Vec<String> {
        match &self.params["path"] {
            Value::String(path) => vec![path.clone()],
            Value::Array(paths) => paths
                .iter()
                .map(|p| p.as_str().unwrap().to_owned())
                .collect(),
            _ => vec![".".into()],
        }
    }
    fn limit(&self) -> usize {
        self.params["limit"].as_u64().unwrap_or(100).max(1) as usize
    }
    fn context(&self) -> (u64, u64) {
        let context = self.params["context"].as_u64().unwrap_or(0);
        (
            self.params["before"].as_u64().unwrap_or(context),
            self.params["after"].as_u64().unwrap_or(context),
        )
    }
}

/// Run the raw engine with the arguments the tool documents.
fn oracle(case: &Case, root: &Path, home: &Path) -> Oracle {
    let p = &case.params;
    let pattern = p["pattern"].as_str().unwrap().to_owned();
    let paths = case.paths();
    let mut records = Records::default();
    let mut unparsed = Vec::new();
    let (before, after) = case.context();
    let (program, mut args): (&str, Vec<String>) = match case.mode {
        Mode::Text => (
            "rg",
            ["--no-config", "--hidden", "--glob", "!.git", "--json"]
                .map(String::from)
                .to_vec(),
        ),
        Mode::Archive => (
            "ugrep",
            [
                "--no-config",
                "-r",
                "-z",
                "-n",
                "-H",
                "-I",
                "--hidden",
                "--ignore-files=.gitignore",
                "--ignore-files=.ignore",
                "--ignore-files=.rgignore",
                "--exclude-dir=.git",
                "--color=never",
                "--null",
            ]
            .map(String::from)
            .to_vec(),
        ),
        Mode::Ast => (
            "ast-grep",
            vec![
                "run".into(),
                "--pattern".into(),
                pattern.clone(),
                "--lang".into(),
                p["language"].as_str().unwrap().into(),
                "--json=stream".into(),
                "--no-ignore".into(),
                "hidden".into(),
            ],
        ),
    };
    if let Some(glob) = p["glob"].as_str() {
        args.push(
            if case.mode == Mode::Ast {
                "--globs"
            } else {
                "--glob"
            }
            .into(),
        );
        args.push(glob.into());
    }
    if case.mode != Mode::Ast {
        if p["literal"].as_bool() == Some(true) {
            args.push("--fixed-strings".into());
        }
        if p["ignoreCase"].as_bool() == Some(true) {
            args.push("--ignore-case".into());
        }
        if before > 0 {
            args.extend(["--before-context".into(), before.to_string()]);
        }
        if after > 0 {
            args.extend(["--after-context".into(), after.to_string()]);
        }
        args.extend(["--regexp".into(), pattern]);
    }
    args.push("--".into());
    args.extend(paths.iter().cloned());
    let output = Command::new(program)
        .args(&args)
        .current_dir(root)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let missing = paths.iter().any(|path| !root.join(path).exists());
    let error = missing
        || match case.mode {
            Mode::Text => output.status.code() == Some(2) && !stderr.contains("(os error"),
            Mode::Archive => output.status.code() == Some(2) && !stderr.contains("warning:"),
            // Exit 1 is "no matches"; the tool rejects any stderr output.
            Mode::Ast => {
                output.status.code().is_none_or(|code| code > 1) || !stderr.trim().is_empty()
            }
        };
    match case.mode {
        Mode::Text => {
            for line in stdout.lines() {
                let event: Value = serde_json::from_str(line).unwrap();
                let kind = event["type"].as_str().unwrap();
                if kind != "match" && kind != "context" {
                    continue;
                }
                let data = &event["data"];
                let key = (
                    normalize_path(data["path"]["text"].as_str().unwrap_or("?")),
                    data["line_number"].as_u64().unwrap(),
                );
                let text = expected_text(data["lines"]["text"].as_str().unwrap_or(""));
                records.add(kind == "match", key, text);
            }
        }
        Mode::Archive => {
            for line in stdout.lines().filter(|line| *line != "--") {
                let Some((path, rest)) = line.split_once('\0') else {
                    unparsed.push(line.to_owned());
                    continue;
                };
                // `path\0{member}:N:text` names an archive member.
                let (member, rest) = match rest.strip_prefix('{') {
                    Some(member) => {
                        let (member, rest) = rest.split_at(member.find('}').unwrap() + 2);
                        (member, &rest[1..])
                    }
                    None => ("", rest),
                };
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                let sep = rest.as_bytes().get(digits).copied().unwrap_or(0);
                if digits == 0 || sep != b':' && sep != b'-' {
                    unparsed.push(line.replace('\0', "\\0"));
                    continue;
                }
                let key = (
                    normalize_path(&format!("{path}{member}")),
                    rest[..digits].parse().unwrap(),
                );
                let text = expected_text(&rest[digits + 1..]);
                records.add(sep == b':', key, text);
            }
        }
        Mode::Ast => {
            for line in stdout.lines().filter(|line| !line.is_empty()) {
                let value: Value = serde_json::from_str(line).unwrap();
                let key = (
                    normalize_path(value["file"].as_str().unwrap()),
                    value["range"]["start"]["line"].as_u64().unwrap() + 1,
                );
                let text = value["text"].as_str().unwrap().replace('\n', "\\n");
                records.add(true, key, expected_text(&text));
            }
        }
    }
    Oracle {
        records,
        error,
        unparsed,
    }
}

fn text_case(rng: &mut Rng) -> Case {
    let patterns = [
        "needle",
        "Needle",
        "needle|plain",
        r"\bfn\s+\w+",
        r"^\s*//",
        r"[0-9]{2}:[0-9]{2}:[0-9]{2}",
        "日本語",
        "café",
        "a.b(c)",
        ";$",
        "zzz_nomatch",
        ".",
        r"impl\s+\w+\s+for",
        "NEEDLE",
        "needle two",
        "(",
        "foo\\(",
        r"line \d+ needle",
    ];
    let globs = [
        "*.rs",
        "!*.md",
        "src/**",
        "**/*.txt",
        "/top.txt",
        "*.{rs,ts}",
        "*.log",
        "docs/*",
    ];
    let paths: Vec<Value> = vec![
        json!("."),
        json!("src"),
        json!("src/nested"),
        json!("docs"),
        json!(["src", "docs"]),
        json!(["top.txt", "src/lib.rs"]),
        json!("top.txt"),
        json!("crlf.txt"),
        json!(".hidden"),
        json!("ignored"),
        json!("long.txt"),
        json!("docs/notes.txt"),
        json!(["noeol.txt", "empty.txt", "a-1-b.txt", "space name.txt"]),
        json!("nope"),
        json!(["src", "nope"]),
    ];
    let mut params = json!({ "pattern": rng.pick(&patterns) });
    if rng.chance(30) {
        params["ignoreCase"] = json!(rng.chance(70));
    }
    if rng.chance(25) {
        params["literal"] = json!(rng.chance(70));
    }
    if rng.chance(30) {
        params["context"] = json!(rng.below(4));
    }
    if rng.chance(25) {
        params["before"] = json!(rng.below(5));
    }
    if rng.chance(25) {
        params["after"] = json!(rng.below(5));
    }
    if rng.chance(60) {
        params["limit"] = json!(rng.pick(&[1, 2, 5, 20, 1000]));
    }
    if rng.chance(30) {
        params["glob"] = json!(rng.pick(&globs));
    }
    if rng.chance(75) {
        params["path"] = rng.pick(&paths);
    }
    Case {
        mode: Mode::Text,
        params,
    }
}

fn archive_case(rng: &mut Rng) -> Case {
    let mut case = text_case(rng);
    case.mode = Mode::Archive;
    case.params["mode"] = json!("archive");
    // ugrep's regex dialect differs from rg's; keep to patterns both share.
    case.params["pattern"] = json!(rng.pick(&[
        "needle",
        "needle|plain",
        "NEEDLE",
        "plain gz",
        "zzz_nomatch"
    ]));
    if rng.chance(40) {
        case.params["path"] = rng.pick(&[
            json!("archive.tar"),
            json!("zipped.txt.gz"),
            json!(["docs", "archive.tar"]),
        ]);
    }
    case
}

fn ast_case(rng: &mut Rng) -> Case {
    let (language, pattern) = rng.pick(&[
        ("rust", "fn $N($$$ARGS) { $$$BODY }"),
        ("rust", "$A.unwrap()"),
        ("rust", "impl $T for $U { $$$ }"),
        ("rust", "foo($A)"),
        ("rust", "let $X = $Y;"),
        ("rust", "pub struct $S;"),
        ("typescript", "console.log($$$)"),
        ("typescript", "const $X = $Y"),
    ]);
    let mut params = json!({ "pattern": pattern, "mode": "ast", "language": language });
    if rng.chance(60) {
        params["limit"] = json!(rng.pick(&[1, 2, 5, 1000]));
    }
    if rng.chance(30) {
        params["glob"] = json!(rng.pick(&["*.rs", "src/nested/**", "!src/main.rs"]));
    }
    if rng.chance(70) {
        params["path"] = rng.pick(&[
            json!("src"),
            json!("src/lib.rs"),
            json!(["src/lib.rs", "src/main.rs"]),
            json!("src/nested"),
            json!("."),
        ]);
    }
    Case {
        mode: Mode::Ast,
        params,
    }
}

/// Parameter combinations every mode must reject.
fn invalid_cases() -> Vec<Value> {
    vec![
        json!({"pattern":"x", "language":"rust"}),
        json!({"pattern":"x", "mode":"archive", "language":"rust"}),
        json!({"pattern":"fn $N() {}", "mode":"ast"}),
        json!({"pattern":"fn $N() {}", "mode":"ast", "language":""}),
        json!({"pattern":"x", "mode":"ast", "language":"rust", "context":1}),
        json!({"pattern":"x", "mode":"ast", "language":"rust", "before":1}),
        json!({"pattern":"x", "mode":"ast", "language":"rust", "after":2}),
        json!({"pattern":"x", "mode":"ast", "language":"rust", "literal":true}),
        json!({"pattern":"x", "mode":"ast", "language":"rust", "ignoreCase":true}),
        json!({"pattern":"x", "mode":"regex"}),
        json!({"pattern":"x", "path":[]}),
        json!({"pattern":"x", "path":42}),
        json!({"path":"."}),
    ]
}

async fn run(tool: &Arc<dyn AgentTool>, params: &Value) -> Result<String, String> {
    tool.execute(
        "matrix",
        params.clone(),
        CancellationToken::new(),
        Arc::new(|_| ()),
    )
    .await
    .map(|result| match &result.content[0] {
        TextContentOrImage::Text(text) => text.text.clone(),
        _ => panic!("expected text"),
    })
    .map_err(|error| error.to_string())
}

/// Compare one engine's output with the oracle; returns failure categories.
fn check(
    case: &Case,
    engine: &str,
    result: &Result<String, String>,
    oracle: &Oracle,
    known: &[String],
    failures: &mut Vec<(String, String)>,
) {
    let mut fail = |category: &str, detail: String| {
        failures.push((
            format!("{engine}: {category}"),
            format!("{} -> {detail}", case.params),
        ));
    };
    if !oracle.unparsed.is_empty() {
        fail("oracle unparsed line", format!("{:?}", oracle.unparsed));
    }
    let output = match (result, oracle.error) {
        (Err(_), true) => return,
        (Err(error), false) => return fail("unexpected error", error.clone()),
        (Ok(output), true) => return fail("missing error", output.chars().take(300).collect()),
        (Ok(output), false) => output,
    };
    let parsed = parse_tool(output, known);
    if !parsed.unparsed.is_empty() {
        fail("unparsed output line", format!("{:?}", parsed.unparsed));
    }
    let expected = &oracle.records;
    let got = &parsed.records;
    let total = expected.match_count();
    let limit = case.limit();
    let limit_notice = parsed.notices.contains("matches limit reached");
    let byte_capped = parsed.notices.contains("KB limit reached")
        || parsed.notices.contains("Search output truncated");
    if total == 0 && !parsed.no_matches {
        fail(
            "expected 'No matches found'",
            output.chars().take(300).collect(),
        );
    }
    // A context line that is also a match is the in-process engine's repeat.
    let context_keys = |records: &Records| -> BTreeSet<Key> {
        records
            .context
            .keys()
            .filter(|key| !expected.matches.contains_key(*key))
            .cloned()
            .collect()
    };
    let in_oracle =
        |key: &Key| expected.matches.contains_key(key) || expected.context.contains_key(key);
    for (key, texts) in got.matches.iter().chain(got.context.iter()) {
        let want: BTreeSet<&String> = expected
            .matches
            .get(key)
            .into_iter()
            .chain(expected.context.get(key))
            .flatten()
            .collect();
        if want.is_empty() {
            fail("line not in oracle", format!("{key:?} {texts:?}"));
        } else if let Some(text) = texts.iter().find(|text| !want.contains(text)) {
            fail(
                "text differs",
                format!("{key:?} got {text:?} want {want:?}"),
            );
        }
    }
    if total > limit {
        if !byte_capped && got.match_count() != limit {
            fail(
                "match count under limit",
                format!("got {} want {limit} (oracle {total})", got.match_count()),
            );
        }
        if !limit_notice && !byte_capped {
            fail("missing limit notice", parsed.notices.clone());
        }
        if !got
            .matches
            .keys()
            .all(|key| expected.matches.contains_key(key))
        {
            fail("match outside oracle", String::new());
        }
        if !got.context.keys().all(in_oracle) {
            fail("context outside oracle", String::new());
        }
    } else if !byte_capped {
        let got_keys: BTreeSet<_> = got.matches.keys().cloned().collect();
        let want_keys: BTreeSet<_> = expected.matches.keys().cloned().collect();
        if got_keys != want_keys || got.match_count() != total {
            fail(
                "match set differs",
                format!(
                    "missing {:?} extra {:?}",
                    want_keys.difference(&got_keys).collect::<Vec<_>>(),
                    got_keys.difference(&want_keys).collect::<Vec<_>>()
                ),
            );
        }
        let got_context = context_keys(got);
        let want_context = context_keys(expected);
        if got_context != want_context {
            fail(
                "context set differs",
                format!(
                    "missing {:?} extra {:?}",
                    want_context.difference(&got_context).collect::<Vec<_>>(),
                    got_context.difference(&want_context).collect::<Vec<_>>()
                ),
            );
        }
        // Reaching the limit exactly may be reported (ice) or not (rg).
        if limit_notice && total < limit {
            fail("spurious limit notice", parsed.notices.clone());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn grep_matrix_matches_engine_oracles() {
    if !installed("rg") {
        eprintln!("grep matrix skipped: rg is not installed");
        return;
    }
    let seed = std::env::var("GREP_MATRIX_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x1ce_9e9_u64);
    let text_cases: usize = std::env::var("GREP_MATRIX_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let archive = installed("ugrep") && installed("tar") && installed("gzip");
    let ast = installed("ast-grep");
    build_fixture(&root, archive);
    let known = known_paths(&root);

    // Hermetic config: no user rg config or global git excludes.
    let mut vars = HashMap::new();
    vars.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    vars.insert(
        "XDG_CONFIG_HOME".to_string(),
        home.to_string_lossy().into_owned(),
    );
    let tool_with = |vars: HashMap<String, String>| -> Arc<dyn AgentTool> {
        let env: Arc<dyn ice_tools::ExecutionEnv> =
            Arc::new(OsExecutionEnv::with_cwd(root.clone()).with_shell_env(vars));
        ice_tools::create_grep_tool(&ExecutionToolContext::new(env, None), None)
    };
    let external = tool_with(vars.clone());
    let mut no_programs = vars.clone();
    no_programs.insert("PATH".into(), home.to_string_lossy().into_owned());
    let in_process = tool_with(no_programs);

    let mut rng = Rng(seed);
    let mut cases: Vec<Case> = (0..text_cases).map(|_| text_case(&mut rng)).collect();
    if archive {
        cases.extend((0..150).map(|_| archive_case(&mut rng)));
    }
    if ast {
        cases.extend((0..150).map(|_| ast_case(&mut rng)));
    }

    let mut failures = Vec::new();
    let mut runs = 0usize;
    let mut limited = 0usize;
    for case in &cases {
        let oracle = oracle(case, &root, &home);
        if oracle.records.match_count() > case.limit() {
            limited += 1;
        }
        let result = run(&external, &case.params).await;
        runs += 1;
        let engine = match case.mode {
            Mode::Text => "rg",
            Mode::Archive => "ugrep",
            Mode::Ast => "ast-grep",
        };
        check(case, engine, &result, &oracle, &known, &mut failures);
        if case.mode == Mode::Text {
            let result = run(&in_process, &case.params).await;
            runs += 1;
            check(case, "in-process", &result, &oracle, &known, &mut failures);
        }
    }
    let invalid = invalid_cases();
    for params in &invalid {
        for (engine, tool) in [("rg", &external), ("in-process", &in_process)] {
            runs += 1;
            if let Ok(output) = run(tool, params).await {
                failures.push((
                    format!("{engine}: invalid input accepted"),
                    format!(
                        "{params} -> {}",
                        output.chars().take(200).collect::<String>()
                    ),
                ));
            }
        }
    }

    let total_cases = cases.len() + invalid.len();
    eprintln!(
        "grep matrix: seed {seed:#x}, {total_cases} cases ({text_cases} text, archive {archive}, ast {ast}, {} invalid), {runs} tool runs, {limited} under limit pressure",
        invalid.len()
    );
    let mut by_category: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (category, detail) in &failures {
        by_category.entry(category).or_default().push(detail);
    }
    let mut report = String::new();
    for (category, details) in &by_category {
        report.push_str(&format!("\n[{}] {category}\n", details.len()));
        for detail in details.iter().take(3) {
            report.push_str(&format!(
                "    {}\n",
                detail.chars().take(600).collect::<String>()
            ));
        }
    }
    assert!(failures.is_empty(), "{} failures:{report}", failures.len());
}
