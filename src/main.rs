//! rime-type-stats：统计 Rime 每日打字字数，提供菜单栏入口与本地仪表盘。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rime_type_stats::server::{bind_first_available, serve};
use rime_type_stats::store::Store;
use rime_type_stats::sync::{sync_once, to_iso_day, SyncState};
use rime_type_stats::tray;

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn rime_dir() -> PathBuf {
    std::env::var_os("RIME_TYPE_STATS_RIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("Library/Rime/stats.userdb"))
}

fn data_dir() -> PathBuf {
    std::env::var_os("RIME_TYPE_STATS_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("Library/Application Support/rime-type-stats"))
}

fn print_days(days: &std::collections::BTreeMap<String, rime_type_stats::rime::parser::DayStats>) {
    println!("共 {} 天有记录", days.len());
    let mut total = 0u64;
    for (day, stats) in days {
        total += stats.characters;
        println!(
            "{day}  字数 {:>7}  击键 {:>8}  上屏 {:>5}  速度 {:>3}",
            stats.characters, stats.keystrokes, stats.commits, stats.speed_cpm
        );
    }
    println!("合计 {total} 字");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let serve_only = args.iter().any(|a| a == "--serve-only");
    let dump = args.iter().any(|a| a == "--dump");

    let data_dir = data_dir();
    let rime_dir = rime_dir();

    if dump {
        match rime_type_stats::rime::parser::read_dir(&rime_dir) {
            Ok(raw) => {
                // 紧凑日期转 ISO 便于阅读
                let converted: std::collections::BTreeMap<_, _> = raw
                    .into_iter()
                    .filter_map(|(k, v)| to_iso_day(&k).map(|iso| (iso, v)))
                    .collect();
                print_days(&converted);
            }
            Err(e) => {
                eprintln!("解析 Rime 数据失败：{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let store = match Store::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法打开数据目录 {}：{e}", data_dir.display());
            std::process::exit(1);
        }
    };

    // 启动即同步一次，让 UI 立刻有数据
    let outcome = sync_once(&rime_dir, &store);
    let initial = store.load().unwrap_or_else(|_| outcome.days.clone());

    if let Some(err) = &outcome.error {
        eprintln!("首次同步告警：{err}");
    }

    let state = SyncState::new(initial);
    state.replace(outcome);

    let (server, port) = match bind_first_available(8787, 11) {
        Some(pair) => pair,
        None => {
            eprintln!("无法在 8787..8797 绑定端口，请检查是否已有实例在运行");
            std::process::exit(1);
        }
    };
    println!("rime-type-stats 已启动：http://127.0.0.1:{port}/");

    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let state = state.clone();
        let shutdown = shutdown.clone();
        std::thread::spawn(move || serve(server, state, shutdown));
    }

    if serve_only {
        // 阻塞等待 Ctrl-C
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    tray::run_app(rime_dir, data_dir, state, port)
}
