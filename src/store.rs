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

impl DayRecord {
    fn stats(&self) -> DayStats {
        DayStats {
            characters: self.characters,
            keystrokes: self.keystrokes,
            commits: self.commits,
            speed_cpm: self.speed_cpm,
        }
    }
}

/// 本机今天的 ISO 日期（`YYYY-MM-DD`）。
fn today_iso() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub struct Store {
    dir: PathBuf,
}

/// 把旧版本的数据目录整体改名到新位置。
///
/// 只在「新目录不存在、旧目录存在」时动作，因此可重复调用。改名是原子的，
/// 失败（例如跨设备）时返回错误且不删除任何数据，调用方决定是否继续。
pub fn migrate_dir(legacy: &Path, current: &Path) -> std::io::Result<()> {
    if current.exists() || !legacy.exists() {
        return Ok(());
    }
    if let Some(parent) = current.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(legacy, current)
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

    /// 读取全部记录，按天折叠。
    pub fn load(&self) -> std::io::Result<BTreeMap<String, DayStats>> {
        self.load_with_today(&today_iso())
    }

    /// 按天折叠，`today` 那天取最后一条记录，其余天取各字段最大值。
    ///
    /// 历史日期取 max：文件轮转瞬间可能只读到部分数据，max 能抵御这种截断。
    /// 当天取最后一条：万象的计数在 Rime 重建统计库或清空写缓存时会回落，
    /// 若这里也取 max，菜单栏就会永远停在历史峰值不动。
    pub fn load_with_today(&self, today: &str) -> std::io::Result<BTreeMap<String, DayStats>> {
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
            let entry = out.entry(record.day.clone()).or_default();
            if record.day == today {
                // 当天以最后写入的值为准
                *entry = record.stats();
                continue;
            }
            entry.characters = entry.characters.max(record.characters);
            entry.keystrokes = entry.keystrokes.max(record.keystrokes);
            entry.commits = entry.commits.max(record.commits);
            entry.speed_cpm = entry.speed_cpm.max(record.speed_cpm);
        }

        Ok(out)
    }

    /// 追加一天的最新累计值；与现有值完全相同时跳过并返回 false。
    ///
    /// `allow_decrease` 用于当天：万象的统计值会在 Rime 重置统计库或重建写缓存时
    /// 回落，此时以源数据为准，否则菜单栏会永远卡在历史峰值。历史日期传 false，
    /// 取最大值以抵御文件轮转瞬间读不全、或旧数据被裁剪导致的截断。
    pub fn append(
        &self,
        day: &str,
        stats: &DayStats,
        allow_decrease: bool,
    ) -> std::io::Result<bool> {
        let current = self.load()?;
        if let Some(existing) = current.get(day) {
            let unchanged = if allow_decrease {
                *existing == *stats
            } else {
                existing.characters >= stats.characters
                    && existing.keystrokes >= stats.keystrokes
                    && existing.commits >= stats.commits
                    && existing.speed_cpm >= stats.speed_cpm
            };
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
        let dir = std::env::temp_dir().join(format!(
            "rime-type-stats-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn migrates_legacy_dir_once() {
        let root = temp_dir("migrate");
        let legacy = root.join("rime-counter");
        let current = root.join("rime-type-stats");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(EVENTS_FILE), b"{\"day\":\"2026-01-01\"}\n").unwrap();

        migrate_dir(&legacy, &current).unwrap();
        assert!(!legacy.exists());
        assert!(current.join(EVENTS_FILE).exists());

        // 新目录已存在时不再动作，也不会覆盖
        migrate_dir(&legacy, &current).unwrap();
        assert!(current.join(EVENTS_FILE).exists());
    }

    #[test]
    fn migrate_is_noop_without_legacy_dir() {
        let root = temp_dir("migrate-none");
        let current = root.join("rime-type-stats");
        migrate_dir(&root.join("rime-counter"), &current).unwrap();
        assert!(!current.exists());
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
        assert!(store.append("2026-11-06", &stats(100), false).unwrap());

        let loaded = store.load().unwrap();
        assert_eq!(loaded["2026-11-06"], stats(100));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_same_value_is_skipped() {
        let dir = temp_dir("skip");
        let store = Store::open(&dir).unwrap();
        let first = store.append("2026-11-06", &stats(100), false).unwrap();
        let second = store.append("2026-11-06", &stats(100), false).unwrap();
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
        assert!(store.append("2026-11-06", &stats(1), false).unwrap());
        assert!(nested.join("events.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 回归：万象的当日计数会回落（Rime 重建统计库 / 清空写缓存），
    /// 若当天也取 max，菜单栏会永远停在历史峰值。
    #[test]
    fn today_uses_last_record_even_when_it_decreases() {
        let dir = temp_dir("today-decrease");
        std::fs::write(
            dir.join("events.jsonl"),
            r#"{"day":"2026-11-09","characters":826,"keystrokes":1668,"commits":492,"speed_cpm":80}
{"day":"2026-11-09","characters":38,"keystrokes":76,"commits":20,"speed_cpm":82}
"#,
        )
        .unwrap();

        let store = Store::open(&dir).unwrap();
        let loaded = store.load_with_today("2026-11-09").unwrap();
        assert_eq!(
            loaded["2026-11-09"].characters, 38,
            "当天必须以最后一条为准，不能取历史峰值"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 历史日期仍取 max：文件轮转瞬间可能读不全，max 抵御截断。
    #[test]
    fn historical_days_still_take_max() {
        let dir = temp_dir("history-max");
        std::fs::write(
            dir.join("events.jsonl"),
            r#"{"day":"2026-11-06","characters":2085,"keystrokes":4217,"commits":1410,"speed_cpm":85}
{"day":"2026-11-06","characters":900,"keystrokes":1800,"commits":600,"speed_cpm":80}
"#,
        )
        .unwrap();

        let store = Store::open(&dir).unwrap();
        let loaded = store.load_with_today("2026-11-09").unwrap();
        assert_eq!(
            loaded["2026-11-06"].characters, 2085,
            "非当天日期应取最大值"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn today_append_accepts_decrease() {
        let dir = temp_dir("today-append");
        let store = Store::open(&dir).unwrap();
        assert!(store.append("2026-11-09", &stats(826), true).unwrap());
        assert!(
            store.append("2026-11-09", &stats(38), true).unwrap(),
            "当天的回落值必须能写入"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_append_rejects_decrease() {
        let dir = temp_dir("history-append");
        let store = Store::open(&dir).unwrap();
        assert!(store.append("2026-11-06", &stats(826), false).unwrap());
        assert!(
            !store.append("2026-11-06", &stats(38), false).unwrap(),
            "历史日期不接受回落值"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
