//! 内部 key 解析与按天聚合。

use std::collections::BTreeMap;
use std::path::Path;

use super::sstable::parse_table;
use super::wal::{parse_log, BatchOp};

/// 每日统计。`characters` 为字数的核心指标。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DayStats {
    pub characters: u64,
    pub keystrokes: u64,
    pub commits: u64,
    pub speed_cpm: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawField {
    Characters,
    Keystrokes,
    CommitLength,
    SpeedAverageChar,
    SpeedAverageMs,
}

#[derive(Debug)]
pub enum ReadError {
    Io(std::io::Error),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "读取目录失败：{e}"),
        }
    }
}

impl std::error::Error for ReadError {}

const PREFIX: &[u8] = b"statistics/day/";
/// 尾部 8 字节：seq(7) + type(1)
const KEY_TRAILER: usize = 8;
/// user key 尾部：空格 + TAB + 8 位 device_id
const DEVICE_SUFFIX: usize = 2 + 8;

/// 解析内部 key，返回 `(YYYYMMDD, 字段类别)`；非统计 key 或未知字段返回 None。
pub fn parse_internal_key(user_key: &[u8]) -> Option<(String, RawField)> {
    let (day, field) = parse_key_parts(user_key)?;
    Some((day.to_string(), classify(field)?))
}

/// 取出 `(YYYYMMDD, 字段原始字节)`，保留完整字段名以便区分 `commit_length/1`、
/// `commit_length/3` 等不同分桶。
///
/// 兼容两种形态：SSTable 中的内部 key（尾部带 `seq(7)+type(1)`）与 WAL 中的
/// user key（无尾部）。
fn parse_key_parts(user_key: &[u8]) -> Option<(&str, &[u8])> {
    parse_key_body(user_key).or_else(|| {
        let body_end = user_key.len().checked_sub(KEY_TRAILER)?;
        parse_key_body(&user_key[..body_end])
    })
}

/// 解析无尾部的 user key。
fn parse_key_body(user_key: &[u8]) -> Option<(&str, &[u8])> {
    if user_key.len() < PREFIX.len() + 8 + 1 + DEVICE_SUFFIX {
        return None;
    }
    if !user_key.starts_with(PREFIX) {
        return None;
    }

    let device = &user_key[user_key.len() - DEVICE_SUFFIX..];
    // 校验分隔符与 device_id 形态
    if device[0] != b' ' || device[1] != b'\t' {
        return None;
    }
    if !device[2..].iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }

    let head = &user_key[PREFIX.len()..user_key.len() - DEVICE_SUFFIX];
    // 形态：YYYYMMDD/field
    if head.len() < 9 || head[8] != b'/' {
        return None;
    }
    let day_bytes = &head[..8];
    if !day_bytes.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let day = std::str::from_utf8(day_bytes).ok()?;
    Some((day, &head[9..]))
}

/// 把字段名映射为统计类别；未知字段返回 None。
fn classify(field: &[u8]) -> Option<RawField> {
    match field {
        b"text/characters" => Some(RawField::Characters),
        b"text/code_keystrokes" => Some(RawField::Keystrokes),
        b"speed_average/characters" => Some(RawField::SpeedAverageChar),
        b"speed_average/milliseconds" => Some(RawField::SpeedAverageMs),
        f if f.starts_with(b"commit_length/") => Some(RawField::CommitLength),
        _ => None,
    }
}

/// 从 `c=<int> d=0 t=0` 中取出 `c`。
fn parse_count(value: &[u8]) -> u64 {
    let text = match std::str::from_utf8(value) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    for part in text.split_whitespace() {
        if let Some(rest) = part.strip_prefix("c=") {
            return rest.parse::<u64>().unwrap_or(0);
        }
    }
    0
}

/// 速度需要两个字段同时存在才能换算，因此按天暂存两个累计值再统一计算。
pub fn aggregate(
    entries: impl IntoIterator<Item = (Vec<u8>, u64, Vec<u8>)>,
) -> BTreeMap<String, DayStats> {
    use std::collections::HashMap;

    // 以完整字段名（如 commit_length/1、commit_length/3）为键分别取最新值，
    // 否则不同分桶会互相覆盖，commits 就无法求和。
    let mut latest: HashMap<(String, Vec<u8>), (u64, u64)> = HashMap::new();
    for (user_key, seq, value) in entries {
        let Some((day, field)) = parse_key_parts(&user_key) else {
            continue;
        };
        let key = (day.to_string(), field.to_vec());
        let amount = parse_count(&value);
        match latest.get(&key) {
            Some((existing_seq, _)) if *existing_seq >= seq => {}
            _ => {
                latest.insert(key, (seq, amount));
            }
        }
    }

    // 先收集每天的原始字段值
    struct Acc {
        characters: u64,
        keystrokes: u64,
        commits: u64,
        speed_chars: u64,
        speed_ms: u64,
    }
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    for ((day, field), (_, amount)) in latest {
        let Some(kind) = classify(&field) else {
            continue;
        };
        let acc = accs.entry(day).or_insert(Acc {
            characters: 0,
            keystrokes: 0,
            commits: 0,
            speed_chars: 0,
            speed_ms: 0,
        });
        match kind {
            RawField::Characters => acc.characters = amount,
            RawField::Keystrokes => acc.keystrokes = amount,
            RawField::CommitLength => acc.commits = acc.commits.saturating_add(amount),
            RawField::SpeedAverageChar => acc.speed_chars = amount,
            RawField::SpeedAverageMs => acc.speed_ms = amount,
        }
    }

    accs.into_iter()
        .map(|(day, acc)| {
            let speed_cpm = if acc.speed_ms > 0 {
                ((acc.speed_chars as u128 * 60_000) / acc.speed_ms as u128) as u32
            } else {
                0
            };
            (
                day,
                DayStats {
                    characters: acc.characters,
                    keystrokes: acc.keystrokes,
                    commits: acc.commits,
                    speed_cpm,
                },
            )
        })
        .collect()
}

/// 扫描目录下的 `*.ldb` 与 `*.log` 并聚合。单个文件损坏不影响其余文件。
pub fn read_dir(dir: &Path) -> Result<BTreeMap<String, DayStats>, ReadError> {
    let mut collected: Vec<(Vec<u8>, u64, Vec<u8>)> = Vec::new();

    let read_dir = std::fs::read_dir(dir).map_err(ReadError::Io)?;
    let mut ldb_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut log_paths: Vec<std::path::PathBuf> = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(ReadError::Io)?;
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("ldb") => ldb_paths.push(path),
            Some("log") => log_paths.push(path),
            _ => {}
        }
    }
    // 文件名即文件编号，按编号升序保证 seq 递增语义
    ldb_paths.sort();
    log_paths.sort();

    for path in &ldb_paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                // 文件可能在读取瞬间被 LevelDB 轮转掉：跳过即可
                eprintln!("跳过 {}：{e}", path.display());
                continue;
            }
        };
        match parse_table(&bytes) {
            Ok(entries) => {
                for entry in entries {
                    let (user_key, seq) = split_internal_key(&entry.key);
                    collected.push((user_key, seq, entry.value));
                }
            }
            Err(e) => {
                // 单个文件损坏不影响整体统计
                eprintln!("跳过 {}：{e}", path.display());
            }
        }
    }

    for path in &log_paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("跳过 {}：{e}", path.display());
                continue;
            }
        };
        for batch in parse_log(&bytes) {
            for (i, op) in batch.ops.into_iter().enumerate() {
                match op {
                    BatchOp::Put { key, value } => {
                        let seq = batch.seq + i as u64;
                        // WAL 中的 key 就是 user key（不带 SSTable 的内部 8 字节尾部），
                        // 直接使用，不能再按内部 key 截断。
                        collected.push((key, seq, value));
                    }
                    BatchOp::Delete { .. } => {
                        // 统计 key 从不删除；即便出现也直接忽略
                    }
                }
            }
        }
    }

    Ok(aggregate(collected))
}

/// 把内部 key 拆成 `(user_key, seq)`；不足 8 字节时视为 seq=0。
fn split_internal_key(internal: &[u8]) -> (Vec<u8>, u64) {
    if internal.len() < KEY_TRAILER {
        return (internal.to_vec(), 0);
    }
    let split = internal.len() - KEY_TRAILER;
    let user = internal[..split].to_vec();
    let tail = &internal[split..];
    let mut seq_bytes = [0u8; 8];
    seq_bytes[..7].copy_from_slice(&tail[..7]);
    (user, u64::from_le_bytes(seq_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_key(day: &str, field: &str) -> Vec<u8> {
        let mut k = format!("statistics/day/{day}/{field}").into_bytes();
        k.extend_from_slice(b" \t");
        k.extend_from_slice(b"9b3d1f36");
        k
    }

    fn internal_key(user: &[u8], seq: u64) -> Vec<u8> {
        let mut k = user.to_vec();
        k.extend_from_slice(&seq.to_le_bytes()[..7]);
        k.push(1);
        k
    }

    fn value(c: u64) -> Vec<u8> {
        format!("c={c} d=0 t=0").into_bytes()
    }

    #[test]
    fn parses_internal_key_fields() {
        let k = internal_key(&user_key("20261106", "text/characters"), 42);
        let (day, field) = parse_internal_key(&k).expect("应能解析");
        assert_eq!(day, "20261106");
        assert_eq!(field, RawField::Characters);

        let k = internal_key(&user_key("20261106", "text/code_keystrokes"), 42);
        assert_eq!(parse_internal_key(&k).unwrap().1, RawField::Keystrokes);

        let k = internal_key(&user_key("20261106", "commit_length/3"), 42);
        assert_eq!(parse_internal_key(&k).unwrap().1, RawField::CommitLength);

        let k = internal_key(&user_key("20261106", "speed_average/characters"), 42);
        assert_eq!(
            parse_internal_key(&k).unwrap().1,
            RawField::SpeedAverageChar
        );

        let k = internal_key(&user_key("20261106", "speed_average/milliseconds"), 42);
        assert_eq!(parse_internal_key(&k).unwrap().1, RawField::SpeedAverageMs);

        // 未知字段与非统计 key 一律忽略
        let k = internal_key(&user_key("20261106", "text/code_characters"), 42);
        assert!(parse_internal_key(&k).is_none());
        assert!(parse_internal_key(b"random/key").is_none());
    }

    #[test]
    fn takes_latest_seq_per_key() {
        let uk = user_key("20261106", "text/characters");
        let entries = vec![
            (internal_key(&uk, 10), 10, value(100)),
            (internal_key(&uk, 30), 30, value(300)),
            (internal_key(&uk, 20), 20, value(200)),
        ];
        let map = aggregate(entries);
        assert_eq!(map["20261106"].characters, 300, "应取 seq 最大的值");
    }

    #[test]
    fn sums_commit_lengths_and_computes_speed() {
        let entries = vec![
            (
                internal_key(&user_key("20261106", "commit_length/1"), 1),
                1,
                value(10),
            ),
            (
                internal_key(&user_key("20261106", "commit_length/3"), 1),
                1,
                value(5),
            ),
            (
                internal_key(&user_key("20261106", "commit_length/5_plus"), 1),
                1,
                value(2),
            ),
            (
                internal_key(&user_key("20261106", "text/characters"), 1),
                1,
                value(1000),
            ),
            (
                internal_key(&user_key("20261106", "speed_average/characters"), 1),
                1,
                value(600),
            ),
            (
                internal_key(&user_key("20261106", "speed_average/milliseconds"), 1),
                1,
                value(600_000),
            ),
        ];
        let map = aggregate(entries);
        let d = &map["20261106"];
        assert_eq!(d.characters, 1000);
        assert_eq!(d.commits, 17);
        // 600 字 / 600000ms = 60 字/分
        assert_eq!(d.speed_cpm, 60);
    }

    #[test]
    fn speed_is_zero_when_milliseconds_missing() {
        let entries = vec![(
            internal_key(&user_key("20261106", "speed_average/characters"), 1),
            1,
            value(600),
        )];
        let map = aggregate(entries);
        assert_eq!(map["20261106"].speed_cpm, 0);
    }

    #[test]
    fn day_without_characters_is_still_recorded() {
        let entries = vec![(
            internal_key(&user_key("20261106", "commit_length/1"), 1),
            1,
            value(3),
        )];
        let map = aggregate(entries);
        assert_eq!(map["20261106"].commits, 3);
        assert_eq!(map["20261106"].characters, 0);
    }

    #[test]
    fn empty_input_gives_empty_map() {
        assert!(aggregate(Vec::new()).is_empty());
    }
}
