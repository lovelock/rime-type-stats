//! 跟踪 input_statistics 的「多流」问题是否已被上游修复。
//!
//! 用法：`cargo run --release --example trace_streams [YYYYMMDD]`
//!
//! 判定方法：把当天该 key 的全部写入按值回归连续段，统计独立累加流数量。
//! 修复前实测（2026-11-09）：1810 次写入、14 条流、峰值 4 条同时活跃。
//! 修复后预期：单条流、无回落。

use std::collections::BTreeMap;

use rime_type_stats::rime::sstable::parse_table;
use rime_type_stats::rime::wal::{parse_log, BatchOp};

fn main() {
    let day = std::env::args()
        .nth(1)
        .unwrap_or_else(|| chrono::Local::now().format("%Y%m%d").to_string());
    let home = std::env::var("HOME").expect("HOME 未设置");
    let dir = std::path::Path::new(&home).join("Library/Rime/stats.userdb");

    let key_suffix = format!("statistics/day/{day}/text/characters");

    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("无法读取 Rime 统计目录")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("ldb") | Some("log")
            )
        })
        .collect();
    paths.sort();

    let mut writes: Vec<(u64, u64)> = Vec::new(); // (seq, value)
    for path in &paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let mut take = |key: &[u8], value: &[u8], seq: u64| {
            let k = String::from_utf8_lossy(key);
            // 必须精确匹配 text/characters，排除 text/code_characters
            if !k.starts_with(&key_suffix) || k.contains("code_characters") {
                return;
            }
            let v = String::from_utf8_lossy(value);
            if let Some(c) = v
                .split_whitespace()
                .find_map(|p| p.strip_prefix("c="))
                .and_then(|n| n.parse::<u64>().ok())
            {
                writes.push((seq, c));
            }
        };

        if name.ends_with(".ldb") {
            if let Ok(entries) = parse_table(&bytes) {
                for e in entries {
                    // SSTable 内部 key 尾部是 seq(7)+type(1)
                    if e.key.len() > 8 {
                        let n = e.key.len() - 8;
                        let mut sb = [0u8; 8];
                        sb[..7].copy_from_slice(&e.key[n..n + 7]);
                        take(&e.key[..n], &e.value, u64::from_le_bytes(sb));
                    }
                }
            }
        } else {
            for b in parse_log(&bytes) {
                for (i, op) in b.ops.iter().enumerate() {
                    if let BatchOp::Put { key, value } = op {
                        take(key, value, b.seq + i as u64);
                    }
                }
            }
        }
    }

    writes.sort_by_key(|(s, _)| *s);

    if writes.is_empty() {
        println!("{day}: 没有找到 text/characters 写入记录");
        return;
    }

    // 按值回落切分独立累加流
    let mut streams: Vec<u64> = Vec::new(); // 各流峰值
    let mut cur_peak = 0u64;
    let mut prev = 0u64;
    let mut falls = 0usize;
    for (_, v) in &writes {
        if *v < prev {
            streams.push(cur_peak);
            cur_peak = 0;
            falls += 1;
        }
        cur_peak = cur_peak.max(*v);
        prev = *v;
    }
    streams.push(cur_peak);

    let last = writes.last().unwrap().1;
    let max = streams.iter().copied().max().unwrap_or(0);

    println!("日期          {day}");
    println!("写入次数      {}", writes.len());
    println!("回落次数      {falls}");
    println!("独立累加流    {}", streams.len());
    println!("最后值        {last}");
    println!("峰值          {max}");
    println!();

    if streams.len() == 1 {
        println!("✅ 单条累加流，未观察到多流交错 —— 修复生效");
    } else {
        println!("❌ 仍有 {} 条交错累加流", streams.len());
        let mut dist: BTreeMap<&str, usize> = BTreeMap::new();
        for _ in 0..streams.len() {
            *dist.entry("流").or_insert(0) += 1;
        }
        println!("   注意：历史数据（修复前的写入）仍留在库里，");
        println!("   只有新产生的写入才能反映修复效果。");
    }
}
