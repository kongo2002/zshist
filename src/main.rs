mod store;

use std::io::{self, BufWriter, Read, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};

use store::{Entry, Row, Store};

const ZSH_INIT: &str = include_str!("zsh_init.zsh");

#[derive(Parser)]
#[command(name = "zshist")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the zsh integration script (eval "$(zshist init)")
    Init,
    /// Append a history entry; command text is read from stdin
    Add {
        #[arg(long)]
        dir: String,
        #[arg(long)]
        exit: i32,
        #[arg(long, default_value_t = 0)]
        ms: i64,
        #[arg(long, default_value_t = 0)]
        ts: i64,
    },
    /// Print entries newest-first for fzf
    List {
        #[arg(long)]
        dir: Option<String>,
    },
    /// Print deduped commands with the given prefix, newest first
    Search {
        #[arg(long)]
        dir: Option<String>,
        #[arg(long, default_value_t = 0)]
        limit: usize,
        prefix: String,
    },
    /// Print the full command text for an entry id
    Get {
        #[arg(long)]
        id: String,
    },
    /// Import a zsh EXTENDED_HISTORY file into an empty history
    Import { file: PathBuf },
}

fn data_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("zshist")
        .join("history.jsonl")
}

fn rel_time(t: i64, now: i64) -> String {
    let ago = now - t;
    if ago < 60 {
        format!("{:2}s ago", ago)
    } else if ago < 3600 {
        format!("{:2}m ago", ago / 60)
    } else if ago < 86400 {
        format!("{:2}h ago", ago / 3600)
    } else if ago < 604800 {
        format!("{:2}d ago", ago / 86400)
    } else {
        format!("{:2}w ago", ago / 604800)
    }
}

fn fmt_dur(ms: i64) -> String {
    if ms <= 0 {
        String::new()
    } else if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{}.{}s", ms / 1000, ms % 1000 / 100)
    } else if ms < 3_600_000 {
        format!("{}m{:02}s", ms / 60_000, ms % 60_000 / 1000)
    } else {
        format!("{}h{:02}m", ms / 3_600_000, ms % 3_600_000 / 60_000)
    }
}

/// Commands matching `prefix`, newest first, deduped by command text.
/// Assumes `rows` is already in append (= time) order; `import` refuses to
/// run against a non-empty history so this invariant always holds.
fn search_rows(rows: &[Row], prefix: &str, dir: Option<&str>, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for row in rows.iter().rev() {
        if let Some(d) = dir
            && row.entry.d != d
        {
            continue;
        }
        if !row.entry.c.starts_with(prefix) || !seen.insert(row.entry.c.clone()) {
            continue;
        }
        out.push(row.entry.c.clone());
        if limit > 0 && out.len() == limit {
            break;
        }
    }
    out
}

fn list_rows<'a>(rows: &'a [Row], dir: Option<&str>) -> Vec<&'a Row> {
    rows.iter()
        .rev()
        .filter(|row| dir.is_none_or(|d| row.entry.d == d))
        .collect()
}

const C_BLUE: &str = "\x1b[34m";
const C_DIM: &str = "\x1b[2m";
const C_RED: &str = "\x1b[31m";
const C_RESET: &str = "\x1b[0m";

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cmd_init() {
    print!("{ZSH_INIT}");
}

fn cmd_add(dir: String, exit: i32, ms: i64, ts: i64) {
    let mut raw = String::new();
    if io::stdin().read_to_string(&mut raw).is_err() {
        return;
    }
    let cmd = raw.trim_end_matches('\n');
    if cmd.trim().is_empty() {
        return;
    }
    let t = if ts == 0 { now_unix() } else { ts };
    let m = ms.max(0);
    let entry = Entry {
        t,
        d: dir,
        x: exit,
        c: cmd.to_string(),
        m,
    };
    if let Err(e) = Store::new(data_path()).append(&[entry]) {
        eprintln!("zshist: {e}");
        std::process::exit(1);
    }
}

fn cmd_list(dir: Option<String>) {
    let rows = match Store::new(data_path()).list() {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("zshist: {e}");
            std::process::exit(1);
        }
    };
    let now = now_unix();
    let stdout = io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    for row in list_rows(&rows, dir.as_deref()) {
        let e = &row.entry;
        let owned;
        let disp: &str = if let Some(i) = e.c.find('\n') {
            owned = format!("{} \u{23ce}", &e.c[..i]);
            &owned
        } else {
            e.c.as_str()
        };
        let (col, reset) = if e.x > 0 { (C_RED, C_RESET) } else { ("", "") };
        let _ = writeln!(
            w,
            "{}\t{}{:>7}{}\t{}{:>8}{}\t{}{}{}",
            row.id,
            C_DIM,
            fmt_dur(e.m),
            C_RESET,
            C_BLUE,
            rel_time(e.t, now),
            C_RESET,
            col,
            disp,
            reset,
        );
    }
}

fn cmd_search(dir: Option<String>, limit: usize, prefix: String) {
    let rows = match Store::new(data_path()).list() {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("zshist: {e}");
            std::process::exit(1);
        }
    };
    let matches = search_rows(&rows, &prefix, dir.as_deref(), limit);
    if matches.is_empty() {
        std::process::exit(1);
    }
    let stdout = io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    for c in matches {
        let _ = writeln!(w, "{c}");
    }
}

fn cmd_get(id: String) {
    match Store::new(data_path()).get(&id) {
        Ok(Some(entry)) => println!("{}", entry.c),
        Ok(None) => std::process::exit(1),
        Err(e) => {
            eprintln!("zshist: {e}");
            std::process::exit(1);
        }
    }
}

/// Parses a zsh EXTENDED_HISTORY line: `: <ts>:<duration>;<command>`.
fn parse_hist_line(line: &str) -> Option<(i64, &str)> {
    let rest = line.strip_prefix(": ")?;
    let (ts, rest) = rest.split_once(':')?;
    let (_duration, cmd) = rest.split_once(';')?;
    let t: i64 = ts.parse().ok()?;
    Some((t, cmd))
}

fn import_history(source: &std::path::Path) -> io::Result<Vec<Entry>> {
    // zsh history files aren't guaranteed valid UTF-8 (locale-encoded or
    // meta-quoted bytes); lossily replace invalid sequences instead of
    // failing outright, same as JSON encoding would do to them anyway.
    let bytes = std::fs::read(source)?;
    let mut entries = Vec::new();
    let mut cur: Option<Entry> = None;
    for raw_line in bytes.split(|&b| b == b'\n') {
        let raw_line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let line = String::from_utf8_lossy(raw_line);
        let line = line.as_ref();
        if let Some(entry) = &mut cur {
            entry.c.push('\n');
            entry.c.push_str(line.strip_suffix('\\').unwrap_or(line));
            if !line.ends_with('\\') {
                entries.push(cur.take().unwrap());
            }
            continue;
        }
        let Some((t, cmd)) = parse_hist_line(line) else {
            continue;
        };
        if cmd.is_empty() {
            continue;
        }
        let stripped = cmd.strip_suffix('\\').unwrap_or(cmd);
        let entry = Entry {
            t,
            d: String::new(),
            x: -1,
            c: stripped.to_string(),
            m: 0,
        };
        if cmd.ends_with('\\') {
            cur = Some(entry);
        } else {
            entries.push(entry);
        }
    }
    if cur.is_some() {
        return Err(io::Error::other(
            "unexpected end of file in multiline entry",
        ));
    }
    Ok(entries)
}

fn cmd_import(file: PathBuf) {
    let store = Store::new(data_path());
    match store.is_empty() {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("zshist: refusing to import into a non-empty history file");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("zshist: {e}");
            std::process::exit(1);
        }
    }
    let entries = match import_history(&file) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("zshist: {e}");
            std::process::exit(1);
        }
    };
    let n = entries.len();
    if let Err(e) = store.append(&entries) {
        eprintln!("zshist: {e}");
        std::process::exit(1);
    }
    println!("imported {n} entries");
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => cmd_init(),
        Command::Add { dir, exit, ms, ts } => cmd_add(dir, exit, ms, ts),
        Command::List { dir } => cmd_list(dir),
        Command::Search { dir, limit, prefix } => cmd_search(dir, limit, prefix),
        Command::Get { id } => cmd_get(id),
        Command::Import { file } => cmd_import(file),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(t: i64, d: &str, c: &str) -> Row {
        Row {
            entry: Entry {
                t,
                d: d.to_string(),
                x: 0,
                c: c.to_string(),
                m: 0,
            },
            id: format!("id{t}"),
        }
    }

    fn fixture() -> Vec<Row> {
        vec![
            row(1, "/a", "git status"),
            row(2, "/a", "git log"),
            row(3, "/b", "git status"),
            row(4, "/a", "git status"),
        ]
    }

    #[test]
    fn search_rows_prefix_newest_first_dedup() {
        let got = search_rows(&fixture(), "git", None, 0);
        assert_eq!(got, vec!["git status", "git log"]);
    }

    #[test]
    fn search_rows_dir_filter() {
        let got = search_rows(&fixture(), "git", Some("/b"), 0);
        assert_eq!(got, vec!["git status"]);
    }

    #[test]
    fn search_rows_limit() {
        let rows = vec![
            row(1, "/a", "echo a"),
            row(2, "/a", "echo b"),
            row(3, "/a", "echo c"),
        ];
        let got = search_rows(&rows, "echo", None, 2);
        assert_eq!(got, vec!["echo c", "echo b"]);
    }

    #[test]
    fn search_rows_empty_prefix_matches_all() {
        let got = search_rows(&fixture(), "", None, 0);
        assert_eq!(got, vec!["git status", "git log"]);
    }

    #[test]
    fn search_rows_no_match() {
        let got = search_rows(&fixture(), "nope", None, 0);
        assert!(got.is_empty());
    }

    #[test]
    fn fmt_dur_table() {
        assert_eq!(fmt_dur(0), "");
        assert_eq!(fmt_dur(-5), "");
        assert_eq!(fmt_dur(1), "1ms");
        assert_eq!(fmt_dur(999), "999ms");
        assert_eq!(fmt_dur(1000), "1.0s");
        assert_eq!(fmt_dur(59_999), "59.9s");
        assert_eq!(fmt_dur(60_000), "1m00s");
        assert_eq!(fmt_dur(3_599_999), "59m59s");
        assert_eq!(fmt_dur(3_600_000), "1h00m");
    }

    #[test]
    fn import_history_parses_single_and_multiline() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("hist");
        std::fs::write(&path, ": 100:0;echo hi\n: 200:0;echo \\\nmultiline\n").unwrap();
        let entries = import_history(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].c, "echo hi");
        assert_eq!(entries[0].t, 100);
        assert_eq!(entries[1].c, "echo \nmultiline");
        assert_eq!(entries[1].t, 200);
    }

    #[test]
    fn import_history_tolerates_invalid_utf8() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("hist");
        let mut bytes = b": 100:0;echo \xff\xfebroken\n".to_vec();
        bytes.extend_from_slice(b": 200:0;echo ok\n");
        std::fs::write(&path, &bytes).unwrap();
        let entries = import_history(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].t, 100);
        assert!(entries[0].c.contains("broken"));
        assert_eq!(entries[1].c, "echo ok");
    }

    #[test]
    fn import_history_skips_empty_commands() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("hist");
        std::fs::write(&path, ": 100:0;\n: 200:0;echo hi\n").unwrap();
        let entries = import_history(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].c, "echo hi");
    }

    #[test]
    fn import_refuses_non_empty_history() {
        let dir = tempfile::TempDir::new().unwrap();
        let history_path = dir.path().join("history.jsonl");
        let store = Store::new(&history_path);
        store
            .append(&[Entry {
                t: 1,
                d: String::new(),
                x: 0,
                c: "existing".to_string(),
                m: 0,
            }])
            .unwrap();
        assert!(!store.is_empty().unwrap());
    }
}
