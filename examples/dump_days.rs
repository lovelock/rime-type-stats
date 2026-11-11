//! 用真实 Rime 统计库做冒烟检查：解析 `~/Library/Rime/stats.userdb` 并打印按天统计。

use rime_type_stats::rime::parser;

fn main() {
    let home = std::env::var("HOME").expect("HOME 未设置");
    let dir = std::path::Path::new(&home).join("Library/Rime/stats.userdb");
    match parser::read_dir(&dir) {
        Ok(days) => {
            println!("解析到 {} 天", days.len());
            let total: u64 = days.values().map(|d| d.characters).sum();
            println!("总字数：{total}");
            if let (Some(first), Some(last)) = (days.keys().next(), days.keys().next_back()) {
                println!("最早日期：{first}  最晚日期：{last}");
            }
            for (day, stats) in days.iter().take(5) {
                println!("{day}: {stats:?}");
            }
        }
        Err(e) => eprintln!("解析失败：{e}"),
    }
}
