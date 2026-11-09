# rime-counter

统计每天在 macOS（Squirrel）上用 Rime 打出的字数。菜单栏常驻显示今日字数，点开查看日 / 周 / 月趋势曲线。

## 原理

直接**只读**解析 `~/Library/Rime/stats.userdb`（LevelDB）中的每日统计键：

- 按文件名打开 `*.ldb` 与 `*.log`，不经过 LevelDB 库，因此不触碰 Squirrel 持有的 `LOCK`
- 不需要复制快照、不加文件锁、不修改任何 Rime 配置
- 内置纯 Rust 的 SSTable 与 WAL 解析器，无第三方压缩依赖

数据来自万象输入法的 `input_statistics` 模块，因此**历史数据从启用那天起自动补齐**，无需等待累积。

本地只在 `~/Library/Application Support/rime-counter/` 保存一份追加式日志 `events.jsonl`。
折叠时历史日期对每个字段取最大值，因此重复同步、乱序同步、进程重启后重放都收敛到相同结果（幂等）。
当天例外：万象的当日计数会在 Rime 重建统计库时回落，所以当天以最后一条记录为准。

## 安装

```bash
git clone <repo> && cd rime-counter
scripts/install.sh
```

安装脚本会编译 release、安装到
`~/Library/Application Support/rime-counter/bin/`、做 ad-hoc 签名，并写入
`~/Library/LaunchAgents/com.lovelock.rime-counter.plist` 注册登录自启。

安装后：

- 菜单栏出现 ⌨ 图标与今日字数
- 左键点击打开仪表盘网页
- 右键菜单：打开统计 / 立即同步 / 打开数据目录 / 退出

重新部署（改完代码后）直接重跑 `scripts/install.sh` 即可，它会先卸载旧的 LaunchAgent。

## 卸载

```bash
scripts/uninstall.sh
```

数据目录会保留。如需彻底清除，手动删除 `~/Library/Application Support/rime-counter`。

## 资源占用

菜单栏常驻进程实测：**CPU 0.0%、4 个线程、phys_footprint 17MB**。

作为对照，一个只有图标、不含任何逻辑的空壳菜单栏程序本身就是 12MB ——
这部分是 AppKit / CoreGraphics 的固有开销，无法进一步降低。

CPU 占用有一条容易踩的坑：AppKit 的 `setTitle` 即使文本没有变化也会把状态栏项
标记为脏并触发整条重绘链路。若在事件循环里无条件调用，会形成
「重绘 → 产生事件 → 再次 setTitle」的自持循环，实测能占满一整个核。
因此标题只在同步完成且文本确实变化时才写入。

## 开发

```bash
cargo test                     # 单元测试（51 个）
cargo run -- --dump            # 打印解析到的每日数据
cargo run -- --serve-only      # 只起 HTTP 服务，便于调试前端
cargo run                      # 完整模式（菜单栏 + 网页）
```

环境变量可覆盖默认路径：

- `RIME_COUNTER_RIME_DIR` — Rime 统计库目录，默认 `~/Library/Rime/stats.userdb`（注意是子目录，`read_dir` 只扫描一层）
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

## 已知限制

**当日字数会跳变，且可能低于当日实际打字量。**

万象的 `input_statistics` 在同一天、同一个 key 上会出现两条交错累加的写入序列，
各自独立计数、交替覆盖同一个 LevelDB key（实测 seq 331621 处 `c=854`，
紧接着 23 次写入落在 `c=263..286`，之后又回到 `c=856`，而 856 = 854 + 2）。

结果是同一天不存在单一的「正确」读数：取 seq 最后一条和取全局最大值都会得到
不同且都无法还原真实总量的数字。这是万象侧的行为，无法从外部修正。

本项目取 **seq 最后一条**，即 Rime 自己认为的当前值。等万象修复后此处可移除。

## 架构

| 文件 | 职责 |
|---|---|
| `src/rime/sstable.rs` | LevelDB SSTable 只读解析 |
| `src/rime/wal.rs` | LevelDB WAL 解析（尾部残缺记录丢弃） |
| `src/rime/parser.rs` | 内部 key 解析与按天聚合 |
| `src/store.rs` | 追加式 JSONL 存储与幂等折叠 |
| `src/sync.rs` | 定时同步编排与共享状态 |
| `src/server.rs` | 本地 HTTP 服务（`std::net` 单线程） |
| `src/tray.rs` | 菜单栏事件循环 |
| `src/web/index.html` | 内嵌仪表盘 |

设计文档见 `docs/superpowers/specs/`，实现计划见 `docs/superpowers/plans/`。