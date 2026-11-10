//! 定时同步编排与线程间共享状态。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::rime::parser::{read_dir, DayStats};
use crate::store::Store;

/// 一次同步的完整结果。
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub days: BTreeMap<String, DayStats>,
    pub changed: usize,
    pub error: Option<String>,
    pub last_sync_unix: u64,
}

/// 线程间共享的只读快照，供 HTTP 与菜单栏读取。
#[derive(Clone)]
pub struct SyncState {
    inner: Arc<RwLock<Snapshot>>,
}

#[derive(Clone, Default)]
struct Snapshot {
    days: BTreeMap<String, DayStats>,
    error: Option<String>,
    last_sync_unix: u64,
}

impl SyncState {
    pub fn new(initial: BTreeMap<String, DayStats>) -> Self {
        SyncState {
            inner: Arc::new(RwLock::new(Snapshot {
                days: initial,
                error: None,
                last_sync_unix: 0,
            })),
        }
    }

    /// 返回 `(日表, 错误信息, 最后同步时间)`。
    pub fn snapshot(&self) -> (BTreeMap<String, DayStats>, Option<String>, u64) {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        (
            guard.days.clone(),
            guard.error.clone(),
            guard.last_sync_unix,
        )
    }

    /// 读取某一天的字数，不克隆整张日表。
    ///
    /// 菜单栏每轮事件循环都要取这个值，用 [`Self::snapshot`] 会整表克隆，
    /// 在没有数据变化时纯属浪费。
    pub fn characters_on(&self, iso_day: &str) -> u64 {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        guard.days.get(iso_day).map(|d| d.characters).unwrap_or(0)
    }

    /// 用新结果替换快照。失败（`error` 非空）时保留原有日表，只更新错误信息。
    pub fn replace(&self, outcome: SyncOutcome) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if outcome.error.is_none() {
            guard.days = outcome.days;
        }
        guard.error = outcome.error;
        guard.last_sync_unix = outcome.last_sync_unix;
    }
}

/// 把紧凑日期 `YYYYMMDD` 转成 `YYYY-MM-DD`。
pub fn to_iso_day(compact: &str) -> Option<String> {
    if compact.len() != 8 || !compact.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "{}-{}-{}",
        &compact[..4],
        &compact[4..6],
        &compact[6..]
    ))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 执行一次完整同步。任何失败都收敛为 `error` 字段，不 panic、不返回 Err。
pub fn sync_once(rime_dir: &Path, store: &Store) -> SyncOutcome {
    let scanned = match read_dir(rime_dir) {
        Ok(days) => days,
        Err(e) => {
            return SyncOutcome {
                days: BTreeMap::new(),
                changed: 0,
                error: Some(format!("{e}")),
                last_sync_unix: now_unix(),
            };
        }
    };

    let mut changed = 0usize;
    let mut write_error: Option<String> = None;

    // 先一次性读出当前折叠值，避免 append 内部对每一天都重新全量 load 文件
    // （否则 41 天就是 41 次全文件读取，随历史增长退化为 O(n²)）。
    let current = match store.load() {
        Ok(days) => days,
        Err(e) => {
            return SyncOutcome {
                days: BTreeMap::new(),
                changed,
                error: Some(format!("读取存储失败：{e}")),
                last_sync_unix: now_unix(),
            };
        }
    };

    // 当天单独处理：万象的计数会回落，必须以源数据为准，否则菜单栏卡在历史峰值
    let today = to_iso_day(&chrono::Local::now().format("%Y%m%d").to_string());

    for (compact_day, stats) in &scanned {
        let Some(iso_day) = to_iso_day(compact_day) else {
            continue;
        };
        let is_today = today.as_deref() == Some(iso_day.as_str());
        // 已收敛的日期直接跳过，不必再进 append
        if let Some(existing) = current.get(&iso_day) {
            let unchanged = if is_today {
                *existing == *stats
            } else {
                existing.characters >= stats.characters
                    && existing.keystrokes >= stats.keystrokes
                    && existing.commits >= stats.commits
                    && existing.speed_cpm >= stats.speed_cpm
            };
            if unchanged {
                continue;
            }
        }
        match store.append(&iso_day, stats, is_today) {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(e) => write_error = Some(format!("写入存储失败：{e}")),
        }
    }

    // 以存储折叠结果为准，保证 UI 与磁盘一致
    let days = match store.load() {
        Ok(days) => days,
        Err(e) => {
            return SyncOutcome {
                days: BTreeMap::new(),
                changed,
                error: Some(format!("读取存储失败：{e}")),
                last_sync_unix: now_unix(),
            };
        }
    };

    SyncOutcome {
        days,
        changed,
        error: write_error,
        last_sync_unix: now_unix(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rime-counter-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn converts_compact_day_to_iso() {
        assert_eq!(to_iso_day("20261106").as_deref(), Some("2026-11-06"));
        assert_eq!(to_iso_day("2026-11-06"), None);
        assert_eq!(to_iso_day("2026110"), None);
        assert_eq!(to_iso_day("abcdefgh"), None);
    }

    #[test]
    fn sync_once_reports_error_for_missing_dir() {
        let dir = temp_dir("missing");
        let store = Store::open(&dir).unwrap();
        let outcome = sync_once(&dir.join("不存在的目录"), &store);
        assert!(outcome.error.is_some(), "目录不存在时应记录错误");
        assert!(outcome.days.is_empty());
        assert_eq!(outcome.changed, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn characters_on_reads_single_day() {
        let mut initial = BTreeMap::new();
        initial.insert(
            "2026-11-09".to_string(),
            DayStats {
                characters: 560,
                ..Default::default()
            },
        );
        let state = SyncState::new(initial);
        assert_eq!(state.characters_on("2026-11-09"), 560);
        assert_eq!(state.characters_on("1999-01-01"), 0, "缺失的日期返回 0");
    }

    #[test]
    fn sync_state_keeps_previous_data_on_error() {
        let mut initial = BTreeMap::new();
        initial.insert(
            "2026-11-06".to_string(),
            DayStats {
                characters: 100,
                ..Default::default()
            },
        );
        let state = SyncState::new(initial);

        state.replace(SyncOutcome {
            days: BTreeMap::new(),
            changed: 0,
            error: Some("读取失败".to_string()),
            last_sync_unix: 1,
        });

        let (days, error, _) = state.snapshot();
        assert_eq!(days["2026-11-06"].characters, 100, "失败不应清空既有数据");
        assert_eq!(error.as_deref(), Some("读取失败"));
    }

    #[test]
    fn sync_state_replaces_on_success() {
        let state = SyncState::new(BTreeMap::new());
        let mut days = BTreeMap::new();
        days.insert(
            "2026-11-07".to_string(),
            DayStats {
                characters: 42,
                ..Default::default()
            },
        );
        state.replace(SyncOutcome {
            days,
            changed: 1,
            error: None,
            last_sync_unix: 99,
        });

        let (days, error, ts) = state.snapshot();
        assert_eq!(days["2026-11-07"].characters, 42);
        assert_eq!(error, None);
        assert_eq!(ts, 99);
    }
}
