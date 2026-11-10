# rime-type-stats

统计每天在 macOS（Squirrel）或 Linux（fcitx5-rime / ibus-rime）上用 Rime 打出的字数。菜单栏 / 系统托盘常驻显示今日字数，点开查看日 / 周 / 月趋势曲线。

## 原理

直接**只读**解析 Rime 的 `stats.userdb`（LevelDB）中的每日统计键：

- macOS：`~/Library/Rime/stats.userdb`
- Linux：`~/.local/share/fcitx5/rime/stats.userdb`，同时会探测 ibus / fcitx 的配置目录

解析时：

- 按文件名打开 `*.ldb` 与 `*.log`，不经过 LevelDB 库，因此不触碰输入法持有的 `LOCK`
- 不需要复制快照、不加文件锁、不修改任何 Rime 配置
- 内置纯 Rust 的 SSTable / WAL / Snappy 解析，无第三方压缩依赖

### 这个库是万象来维护的

`stats.userdb` 由 [万象拼音](https://github.com/amzxyz/rime-wanxiang) 的
`input_statistics` 模块创建和写入，本项目只是它的**只读消费者**：

| 项 | 值 | 出处 |
|---|---|---|
| 模块 | `lua/wanxiang/input_statistics.lua` | [源码](https://github.com/amzxyz/rime-wanxiang/blob/wanxiang/lua/wanxiang/input_statistics.lua) |
| 底层封装 | `lua/wanxiang/userdb.lua`（LevelDb 包装 + 弱引用池） | [源码](https://github.com/amzxyz/rime-wanxiang/blob/wanxiang/lua/wanxiang/userdb.lua) |
| 库名 | 配置项 `input_stats/db_name`，默认 `stats` → `stats.userdb` | `input_statistics.lua` 中 `env.stats_db_name` |
| key 形态 | `statistics/day/YYYYMMDD/<field> \t <8 位 device_id>` | 本仓库 `src/rime/parser.rs` |
| 字段 | `text/characters`、`text/code_keystrokes`、`speed_average/characters`、`speed_average/milliseconds`、`commit_length/1..5_plus` | `input_statistics.lua` 的字段映射表 |
| value 形态 | `c=<累计值> d=0 t=0` | 同上 |

因为数据源在万象侧，**历史数据从启用那天起自动补齐**，无需等待本工具累积；
同时万象改字段名或改统计算法时，本工具也会跟着失效（见「已知限制」）。

本地只在数据目录保存一份追加式日志 `events.jsonl`：

- macOS：`~/Library/Application Support/rime-type-stats/`
- Linux：`~/.local/share/rime-type-stats/`

折叠时历史日期对每个字段取最大值，因此重复同步、乱序同步、进程重启后重放都收敛到相同结果（幂等）。
当天例外：万象的当日计数会在 Rime 重建统计库时回落，所以当天以最后一条记录为准。

## 安装

### 从 DMG 安装（推荐）

从 [Releases](https://github.com/lovelock/rime-type-stats/releases) 下载
`rime-type-stats_<版本>_arm64.dmg`，打开后把 **Rime Type Stats** 拖进「应用程序」。

只支持 Apple Silicon（arm64）。

应用做了 ad-hoc 签名（没有 Developer ID），首次打开会被 Gatekeeper 拦下。
任选其一放行：

- 在「应用程序」里右键点图标 →「打开」→ 确认打开
- 或打开后在「系统设置 → 隐私与安全性」里点「仍要打开」

首次放行后不再提示。

登录自启：打开「系统设置 → 通用 → 登录项」，点「+」添加
「应用程序」里的 Rime Type Stats。

### 从源码安装

```bash
git clone https://github.com/lovelock/rime-type-stats && cd rime-type-stats
scripts/install.sh
```

`scripts/install.sh` 按平台分派到对应的安装脚本。

### macOS

安装脚本会编译 release、安装到
`~/Library/Application Support/rime-type-stats/bin/`、做 ad-hoc 签名，并写入
`~/Library/LaunchAgents/com.lovelock.rime-type-stats.plist` 注册登录自启。

安装后（两种方式相同）：

- 菜单栏出现 ⌨ 图标与今日字数
- 左键点击打开仪表盘网页
- 右键菜单：打开统计 / 立即同步 / 打开数据目录 / 退出

重新部署（改完代码后）直接重跑 `scripts/install.sh` 即可，它会先卸载旧的 LaunchAgent。

### Linux

编译依赖：GTK3 开发包（Arch：`gtk3`；Debian/Ubuntu：`libgtk-3-dev`）。
运行时还需要 AppIndicator（Arch：`libappindicator-gtk3` 或 `libayatana-appindicator`；
Debian/Ubuntu：`libayatana-appindicator3-1`）。

安装脚本会编译 release、安装到 `~/.local/share/rime-type-stats/bin/`，写入
`~/.config/systemd/user/rime-type-stats.service`，注册为 systemd 用户服务并立即启动。

安装后：

- 托盘出现蓝色键盘图标
- 点开菜单：第一项「今日 N 字」，以及打开统计 / 立即同步 / 打开数据目录 / 退出
- 查看日志：`journalctl --user -u rime-type-stats -f`

**Linux 托盘差异**：AppIndicator 不支持 macOS 式的状态栏标题，今日字数放在菜单第一项，
需要点开图标查看；网页仪表盘不受影响。

重新部署（改完代码后）直接重跑 `scripts/install.sh` 即可。

## 发布

推送 `v*` 标签即自动构建 DMG 并创建 GitHub Release：

```bash
git tag v0.1.0
git push origin v0.1.0
```

版本号取标签去掉 `v`，写入 `Info.plist` 的 `CFBundleShortVersionString`。
构建在 `macos-15`（Apple Silicon）runner 上完成，`scripts/bundle.sh` 会断言
`uname -m == arm64`，避免产出 x86 包。

本地复现同一条打包流程：

```bash
scripts/bundle.sh 0.1.0   # 产物：dist/rime-type-stats_0.1.0_arm64.dmg
```

应用图标 `assets/AppIcon.icns` 由 `scripts/gen_icns.py` 生成（只依赖 Python
标准库 + `iconutil`），改图形后重跑即可，产物需提交。

`main` 分支的每次推送与 PR 会跑 `.github/workflows/ci.yml`：`cargo fmt --check`、
`cargo clippy -D warnings`、`cargo test`。

## 卸载

```bash
scripts/uninstall.sh
```

数据目录会保留。如需彻底清除：

- macOS：手动删除 `~/Library/Application Support/rime-type-stats`
- Linux：手动删除 `~/.local/share/rime-type-stats`

## 资源占用（macOS）

菜单栏常驻进程实测：**CPU 0.0%、4 个线程、phys_footprint 17MB**。

作为对照，一个只有图标、不含任何逻辑的空壳菜单栏程序本身就是 12MB ——
这部分是 AppKit / CoreGraphics 的固有开销，无法进一步降低。

CPU 占用有一条容易踩的坑：AppKit 的 `setTitle` 即使文本没有变化也会把状态栏项
标记为脏并触发整条重绘链路。若在事件循环里无条件调用，会形成
「重绘 → 产生事件 → 再次 setTitle」的自持循环，实测能占满一整个核。
因此标题只在同步完成且文本确实变化时才写入。

## 开发

```bash
cargo test                     # 单元测试
cargo run -- --dump            # 打印解析到的每日数据
cargo run -- --serve-only      # 只起 HTTP 服务，便于调试前端
cargo run                      # 完整模式（托盘 + 网页）
```

环境变量可覆盖默认路径：

- `RIME_TYPE_STATS_RIME_DIR` — Rime 统计库目录。macOS 默认 `~/Library/Rime/stats.userdb`；
  Linux 自动探测 fcitx5 / ibus / fcitx 的用户目录（注意是 `stats.userdb` 子目录，`read_dir` 只扫描一层）
- `RIME_TYPE_STATS_DATA_DIR` — 数据目录。macOS 默认 `~/Library/Application Support/rime-type-stats`；
  Linux 默认 `~/.local/share/rime-type-stats`

## 仪表盘

`http://127.0.0.1:8787/`（端口被占用时自动向后寻找 8787–8797）

- KPI 行：今日 / 本周 / 本月 / 累计 / 平均速度
- 主图：日 / 周 / 月 × 近 30 天 / 90 天 / 一年 / 全部，hover 十字准线与浮层
- 表格视图开关（无障碍兜底）
- 深色优先，可手动切换浅色

单文件 HTML，无框架、无 CDN、无外部字体，完全离线可用。

<img width="257" height="27" alt="菜单栏截图" src="https://github.com/user-attachments/assets/295e132b-3de6-4ce2-88c5-2073aadd4084" />

<img width="1284" height="795" alt="仪表盘截图" src="https://github.com/user-attachments/assets/8c7b7971-8a6b-4b45-a8dd-779ac73f0fa4" />

## 前提

安装并启用 [万象拼音](https://github.com/amzxyz/rime-wanxiang)，且其
`input_statistics` 模块处于开启状态（macOS 用户目录为 `~/Library/Rime`，
Linux 通常为 `~/.local/share/fcitx5/rime`）。以万象 Pro 版为例，`wanxiang_pro.schema.yaml`
的 `engine/translators` 中应包含：

```yaml
lua_translator@*wanxiang.input_statistics
```

模块会在 Rime 用户目录下创建 `stats.userdb`。库名取自万象配置项
`input_stats/db_name`，默认 `stats`；若你改过这一项，需要同步设置本工具的
`RIME_TYPE_STATS_RIME_DIR`。

未启用时没有统计数据可读。万象的 Base / Pro 版本内置该模块，Lite / Pure 版本不含 Lua。

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
| `src/paths.rs` | 跨平台路径解析（XDG / macOS 目录探测） |
| `src/tray.rs` | 菜单栏 / 系统托盘事件循环 |
| `src/web/index.html` | 内嵌仪表盘 |

设计文档见 `docs/superpowers/specs/`，实现计划见 `docs/superpowers/plans/`。