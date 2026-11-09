//! 追加式 JSONL 存储。
//!
//! 每次同步写入的是“当天截至目前的累计值”，因此折叠时对同一天取各字段最大值，
//! 使重复/乱序/重放同步都收敛到相同结果（幂等）。

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::rime::parser::DayStats;

const EVENTS_FILE: &str = "events.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayRecord {
    pub day: String,
    pub characters: u64,
    pub keystrokes: u64,
    pub commits: u64,
    pub speed_cpm: u32,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// 打开（必要时创建）数据目录。
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Store {
            dir: dir.to_path_buf(),
        })
    }

    fn events_path(&self) -> PathBuf {
        self.dir.join(EVENTS_FILE)
    }

    /// 读取全部记录，按天折叠（各字段取最大）。
    pub fn load(&self) -> std::io::Result<BTreeMap<String, DayStats>> {
        let mut out: BTreeMap<String, DayStats> = BTreeMap::new();
        let content = match std::fs::read_to_string(self.events_path()) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: DayRecord = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(_) => continue, // 坏行跳过，不影响其余数据
            };
            let entry = out.entry(record.day).or_default();
            entry.characters = entry.characters.max(record.characters);
            entry.keystrokes = entry.keystrokes.max(record.keystrokes);
            entry.commits = entry.commits.max(record.commits);
            entry.speed_cpm = entry.speed_cpm.max(record.speed_cpm);
        }

        Ok(out)
    }

    /// 追加一天的最新累计值；与现有值完全相同时跳过并返回 false。
    pub fn append(&self, day: &str, stats: &DayStats) -> std::io::Result<bool> {
        let current = self.load()?;
        if let Some(existing) = current.get(day) {
            let unchanged = existing.characters >= stats.characters
                && existing.keystrokes >= stats.keystrokes
                && existing.commits >= stats.commits
                && existing.speed_cpm >= stats.speed_cpm;
            if unchanged {
                return Ok(false);
            }
        }

        let record = DayRecord {
            day: day.to_string(),
            characters: stats.characters,
            keystrokes: stats.keystrokes,
            commits: stats.commits,
            speed_cpm: stats.speed_cpm,
        };
        let mut line = serde_json::to_string(&record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        std::fs::create_dir_all(&self.dir)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;
        file.write_all(line.as_bytes())?;
        file.flush()?;

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(chars: u64) -> DayStats {
        DayStats {
            characters: chars,
            keystrokes: chars * 3,
            commits: 1,
            speed_cpm: 50,
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rime-counter-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_store_loads_empty_map() {
        let dir = temp_dir("empty");
        let store = Store::open(&dir).unwrap();
        assert!(store.load().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_then_load_roundtrips() {
        let dir = temp_dir("roundtrip");
        let store = Store::open(&dir).unwrap();
        assert!(store.append("2026-11-06", &stats(100)).unwrap());

        let loaded = store.load().unwrap();
        assert_eq!(loaded["2026-11-06"], stats(100));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_same_value_is_skipped() {
        let dir = temp_dir("skip");
        let store = Store::open(&dir).unwrap();
        let first = store.append("2026-11-06", &stats(100)).unwrap();
        let second = store.append("2026-11-06", &stats(100)).unwrap();
        assert!(first, "首次写入应返回 true");
        assert!(!second, "值未变化应返回 false，不追加");

        let content = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert_eq!(content.lines().count(), 1, "文件中只应有一行");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_takes_max_across_duplicate_days() {
        let dir = temp_dir("max");
        std::fs::write(
            dir.join("events.jsonl"),
            r#"{"day":"2026-11-06","characters":100,"keystrokes":300,"commits":1,"speed_cpm":50}
{"day":"2026-11-06","characters":250,"keystrokes":700,"commits":3,"speed_cpm":60}
{"day":"2026-11-06","characters":180,"keystrokes":500,"commits":2,"speed_cpm":55}
"#,
        )
        .unwrap();

        let store = Store::open(&dir).unwrap();
        let loaded = store.load().unwrap();
        let d = &loaded["2026-11-06"];
        assert_eq!(d.characters, 250);
        assert_eq!(d.keystrokes, 700);
        assert_eq!(d.commits, 3);
        assert_eq!(d.speed_cpm, 60);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_skips_malformed_lines() {
        let dir = temp_dir("malformed");
        std::fs::write(
            dir.join("events.jsonl"),
            r#"{"day":"2026-11-06","characters":100,"keystrokes":300,"commits":1,"speed_cpm":50}
这不是 JSON
{"day":"2026-11-07","characters":20,"keystrokes":60,"commits":1,"speed_cpm":40}
"#,
        )
        .unwrap();

        let store = Store::open(&dir).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2, "坏行应被跳过，其余两行正常解析");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_creates_parent_dirs() {
        let dir = temp_dir("nested");
        let nested = dir.join("a").join("b");
        let store = Store::open(&nested).unwrap();
        assert!(store.append("2026-11-06", &stats(1)).unwrap());
        assert!(nested.join("events.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
