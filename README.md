# rime-counter

统计每天在 macOS（Squirrel）上用 Rime 打出的字数。菜单栏常驻显示今日字数，点开查看日 / 周 / 月趋势曲线。

## 原理

直接**只读**解析 `~/Library/Rime/stats.userdb`（LevelDB）中的每日统计键：

- 按文件名打开 `*.ldb` 与 `*.log`，不经过 LevelDB 库，因此不触碰 Squirrel 持有的 `LOCK`
- 不需要复制快照、不加文件锁、不修改任何 Rime 配置
- 内置纯 Rust 的 SSTable 与 WAL 解析器，无第三方压缩依赖

数据来自万象输入法的 `input_statistics` 模块，因此**历史数据从启用那天起自动补齐**，无需等待累积。

本地只在 `~/Library/Application Support/rime-counter/` 保存一份追加式日志 `events.jsonl`。
折叠时按天对每个字段取最大值，因此重复同步、乱序同步、进程重启后重放都收敛到相同结果（幂等）。

## 安装

```bash
git clone <repo> && cd rime-counter
scripts/install.sh
```

安装后：

- 菜单栏出现 ⌨ 图标与今日字数
- 左键点击打开仪表盘网页
- 右键菜单：打开统计 / 立即同步 / 打开数据目录 / 退出
- 已通过 LaunchAgent 注册为登录自启

## 卸载

```bash
scripts/uninstall.sh
```

数据目录会保留。如需彻底清除，手动删除 `~/Library/Application Support/rime-counter`。

## 开发

```bash
cargo test                     # 单元测试（43 个）
cargo run -- --dump            # 打印解析到的每日数据
cargo run -- --serve-only      # 只起 HTTP 服务，便于调试前端
cargo run                      # 完整模式（菜单栏 + 网页）
```

环境变量可覆盖默认路径：

- `RIME_COUNTER_RIME_DIR` — Rime 用户目录，默认 `~/Library/Rime`
- `RIME_COUNTER_DATA_DIR` — 数据目录，默认 `~/Library/Application Support/rime-counter`

## 仪表盘

`http://127.0.0.1:8787/`（端口被占用时自动向后寻找 8787–8797）

- KPI 行：今日 / 本周 / 本月 / 累计 / 平均速度
- 主图：日 / 周 / 月 × 近 30 天 / 90 天 / 一年 / 全部，hover 十字准线与浮层
- 表格视图开关（无障碍兜底）
- 深色优先，可手动切换浅色

单文件 HTML，无框架、无 CDN、无外部字体，完全离线可用。

## 前提

`~/Library/Rime` 下已启用万象的 `input_statistics`，即 `wanxiang_pro.schema.yaml` 中包含：

```yaml
lua_translator@*wanxiang.input_statistics
```

未启用时没有统计数据可读。

## 架构

| 文件 | 职责 |
|---|---|
| `src/rime/sstable.rs` | LevelDB SSTable 只读解析 |
| `src/rime/wal.rs` | LevelDB WAL 解析（尾部残缺记录丢弃） |
| `src/rime/parser.rs` | 内部 key 解析与按天聚合 |
| `src/store.rs` | 追加式 JSONL 存储与幂等折叠 |
| `src/sync.rs` | 定时同步编排与共享状态 |
| `src/server.rs` | 本地 HTTP 服务 |
| `src/tray.rs` | 菜单栏事件循环 |
| `src/web/index.html` | 内嵌仪表盘 |

设计文档见 `docs/superpowers/specs/`，实现计划见 `docs/superpowers/plans/`。