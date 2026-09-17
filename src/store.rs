use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use fd_lock::RwLock;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

pub const MAX_LINE_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub t: i64,
    #[serde(default)]
    pub d: String,
    pub x: i32,
    pub c: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub m: i64,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub entry: Entry,
    pub id: String,
}

struct StoredRow {
    entry: Entry,
    id: String,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Store { path: path.into() }
    }

    fn lock_path(&self) -> PathBuf {
        let mut p = self.path.clone().into_os_string();
        p.push(".lock");
        PathBuf::from(p)
    }

    fn with_lock<T>(&self, exclusive: bool, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.lock_path())?;
        let mut lock = RwLock::new(file);
        if exclusive {
            let _guard = lock.write().map_err(io::Error::other)?;
            f()
        } else {
            let _guard = lock.read().map_err(io::Error::other)?;
            f()
        }
    }

    pub fn append(&self, entries: &[Entry]) -> io::Result<()> {
        let mut encoded = Vec::with_capacity(entries.len());
        for entry in entries {
            encoded.push(encode_entry(entry)?);
        }
        if encoded.is_empty() {
            return Ok(());
        }

        self.with_lock(true, || {
            let mut file = OpenOptions::new()
                .append(true)
                .create(true)
                .read(true)
                .open(&self.path)?;
            let original_size = file.metadata()?.len();

            let rollback = |file: &File| {
                let _ = file.set_len(original_size);
            };

            if original_size > 0 {
                let mut last = [0u8; 1];
                file.seek(SeekFrom::Start(original_size - 1))?;
                file.read_exact(&mut last)?;
                if last[0] != b'\n' {
                    let trailing_size = trailing_line_size(&mut file, original_size)?;
                    if trailing_size >= MAX_LINE_SIZE as u64 {
                        return Err(io::Error::other(format!(
                            "cannot append newline to trailing line of at least {} bytes; limit is {}",
                            trailing_size, MAX_LINE_SIZE
                        )));
                    }
                    file.seek(SeekFrom::End(0))?;
                    if let Err(e) = file.write_all(b"\n") {
                        rollback(&file);
                        return Err(e);
                    }
                }
            }

            for line in &encoded {
                if let Err(e) = file.write_all(line) {
                    rollback(&file);
                    return Err(e);
                }
            }
            Ok(())
        })
    }

    pub fn list(&self) -> io::Result<Vec<Row>> {
        let rows = self.with_lock(false, || self.read_all())?;
        Ok(rows
            .into_iter()
            .map(|r| Row {
                entry: r.entry,
                id: r.id,
            })
            .collect())
    }

    pub fn get(&self, id: &str) -> io::Result<Option<Entry>> {
        let Some((offset, hash)) = parse_id(id) else {
            return Ok(None);
        };

        if let Some(entry) = self.get_at(offset, &hash)? {
            return Ok(Some(entry));
        }

        let rows = self.with_lock(false, || self.read_all())?;
        for row in rows.into_iter().rev() {
            if content_hash(&row.entry) == hash {
                return Ok(Some(row.entry));
            }
        }
        Ok(None)
    }

    fn get_at(&self, offset: u64, hash: &str) -> io::Result<Option<Entry>> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(offset)).is_err() {
            return Ok(None);
        }
        let mut line = Vec::new();
        let mut limited = reader.take(MAX_LINE_SIZE as u64 + 1);
        limited.read_until(b'\n', &mut line)?;
        if line.is_empty() || line.len() > MAX_LINE_SIZE {
            return Ok(None);
        }
        let Ok(entry) = serde_json::from_slice::<Entry>(&line) else {
            return Ok(None);
        };
        if entry.c.is_empty() || content_hash(&entry) != hash {
            return Ok(None);
        }
        Ok(Some(entry))
    }

    fn read_all(&self) -> io::Result<Vec<StoredRow>> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut reader = BufReader::new(file);
        let mut rows = Vec::new();
        let mut offset: u64 = 0;
        let mut line_number = 0usize;
        loop {
            let mut line = Vec::new();
            let n = reader.read_until(b'\n', &mut line)?;
            if n == 0 {
                break;
            }
            line_number += 1;
            if line.len() > MAX_LINE_SIZE {
                return Err(io::Error::other(format!(
                    "read {} line {}: encoded JSON line is {} bytes; limit is {}",
                    self.path.display(),
                    line_number,
                    line.len(),
                    MAX_LINE_SIZE
                )));
            }
            let entry: Entry = serde_json::from_slice(&line).map_err(|e| {
                io::Error::other(format!(
                    "read {} line {}: {}",
                    self.path.display(),
                    line_number,
                    e
                ))
            })?;
            if entry.c.is_empty() {
                return Err(io::Error::other(format!(
                    "read {} line {}: empty command",
                    self.path.display(),
                    line_number
                )));
            }
            let id = make_id(offset, &entry);
            offset += line.len() as u64;
            rows.push(StoredRow { entry, id });
        }
        Ok(rows)
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.list()?.is_empty())
    }
}

fn trailing_line_size(file: &mut File, size: u64) -> io::Result<u64> {
    const CHUNK_SIZE: u64 = 32 * 1024;
    let mut buf = vec![0u8; CHUNK_SIZE as usize];
    let mut total: u64 = 0;
    let mut end = size;
    while end > 0 {
        let start = end.saturating_sub(CHUNK_SIZE);
        let n = (end - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buf[..n])?;
        if let Some(i) = buf[..n].iter().rposition(|&b| b == b'\n') {
            return Ok(total + (n - i - 1) as u64);
        }
        total += n as u64;
        if total >= MAX_LINE_SIZE as u64 {
            return Ok(total);
        }
        end = start;
    }
    Ok(total)
}

fn encode_entry(entry: &Entry) -> io::Result<Vec<u8>> {
    if entry.c.is_empty() {
        return Err(io::Error::other("empty command"));
    }
    let mut encoded = serde_json::to_vec(entry).map_err(io::Error::other)?;
    encoded.push(b'\n');
    if encoded.len() > MAX_LINE_SIZE {
        return Err(io::Error::other(format!(
            "encoded JSON line for entry {} is {} bytes; limit is {}",
            content_hash(entry),
            encoded.len(),
            MAX_LINE_SIZE
        )));
    }
    Ok(encoded)
}

fn content_hash(entry: &Entry) -> String {
    let data = format!(
        "{}\0{}\0{}\0{}\0{}",
        entry.t, entry.d, entry.x, entry.c, entry.m
    );
    let digest = Sha1::digest(data.as_bytes());
    hex_encode(&digest[..6])
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

fn hex_decode_len(s: &str) -> Option<usize> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(s.len() / 2)
}

fn to_base36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap()
}

fn from_base36(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut n: u64 = 0;
    for c in s.chars() {
        let digit = c.to_digit(36)?;
        n = n.checked_mul(36)?.checked_add(digit as u64)?;
    }
    Some(n)
}

pub fn make_id(offset: u64, entry: &Entry) -> String {
    format!("{}-{}", to_base36(offset), content_hash(entry))
}

pub fn parse_id(id: &str) -> Option<(u64, String)> {
    let (offset_text, hash) = id.split_once('-')?;
    if offset_text.is_empty() || hash.len() != 12 {
        return None;
    }
    hex_decode_len(hash).filter(|&len| len == 6)?;
    let offset = from_base36(offset_text)?;
    Some((offset, hash.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(t: i64, d: &str, x: i32, c: &str, m: i64) -> Entry {
        Entry {
            t,
            d: d.to_string(),
            x,
            c: c.to_string(),
            m,
        }
    }

    fn store_in(dir: &TempDir) -> Store {
        Store::new(dir.path().join("history.jsonl"))
    }

    #[test]
    fn append_then_list_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        let entries = vec![
            entry(1, "/a", 0, "echo one", 10),
            entry(2, "/b", 1, "line1\nline2", 0),
            entry(3, "", -1, "echo three", 999),
        ];
        store.append(&entries).unwrap();
        let rows = store.list().unwrap();
        assert_eq!(rows.len(), 3);
        for (row, e) in rows.iter().zip(entries.iter()) {
            assert_eq!(&row.entry, e);
            assert!(!row.id.is_empty());
        }
        let ids: std::collections::HashSet<_> = rows.iter().map(|r| r.id.clone()).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn append_adds_missing_trailing_newline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("history.jsonl");
        fs::write(&path, br#"{"t":1,"d":"","x":0,"c":"first"}"#).unwrap();
        let store = Store::new(&path);
        store.append(&[entry(2, "", 0, "second", 0)]).unwrap();
        let rows = store.list().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entry.c, "first");
        assert_eq!(rows[1].entry.c, "second");
    }

    #[test]
    fn append_rejects_oversized_entry_without_mutating_file() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.append(&[entry(1, "", 0, "valid", 0)]).unwrap();
        let before = fs::read(dir.path().join("history.jsonl")).unwrap();
        let huge = "x".repeat(MAX_LINE_SIZE + 1);
        let err = store.append(&[entry(2, "", 0, &huge, 0)]);
        assert!(err.is_err());
        let after = fs::read(dir.path().join("history.jsonl")).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn get_direct_seek_ignores_later_corruption() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.append(&[entry(1, "/a", 0, "echo one", 0)]).unwrap();
        let rows = store.list().unwrap();
        let id = rows[0].id.clone();

        let path = dir.path().join("history.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"not json garbage\n").unwrap();

        let got = store.get(&id).unwrap();
        assert_eq!(got, Some(rows[0].entry.clone()));
    }

    #[test]
    fn get_returns_none_on_hash_mismatch_at_offset() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.append(&[entry(1, "/a", 0, "echo one", 0)]).unwrap();
        let rows = store.list().unwrap();
        let id = rows[0].id.clone();

        let path = dir.path().join("history.jsonl");
        fs::write(&path, br#"{"t":9,"d":"","x":0,"c":"different"}"#).unwrap();

        let got = store.get(&id).unwrap();
        assert_eq!(got, None);
    }

    #[test]
    fn exit_status_and_duration_distinguish_ids() {
        let e1 = entry(1, "/a", 0, "same", 0);
        let e2 = entry(1, "/a", 1, "same", 0);
        let e3 = entry(1, "/a", 0, "same", 5);
        assert_ne!(make_id(0, &e1), make_id(0, &e2));
        assert_ne!(make_id(0, &e1), make_id(0, &e3));
    }

    #[test]
    fn parse_id_roundtrip() {
        let e = entry(1, "/a", 0, "cmd", 0);
        let id = make_id(42, &e);
        let (offset, hash) = parse_id(&id).unwrap();
        assert_eq!(offset, 42);
        assert_eq!(hash, content_hash(&e));
    }

    #[test]
    fn parse_id_rejects_malformed() {
        assert!(parse_id("").is_none());
        assert!(parse_id("no-dash-missing").is_none());
        assert!(parse_id("1-shorthash").is_none());
        assert!(parse_id("-abcdef012345").is_none());
    }
}
