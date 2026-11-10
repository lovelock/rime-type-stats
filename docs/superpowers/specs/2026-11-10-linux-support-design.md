# rime-counter Linux 兼容设计

日期：2026-11-10
状态：已确认

## 1. 目标

在保留 macOS 现有行为的前提下，让 rime-counter 在 Linux（以 fcitx5-rime 为主）上可用：

- 从 Linux 的 Rime 用户目录读取同一份 `stats.userdb` 每日统计
- 系统托盘常驻，菜单第一项显示今日字数
- 本地网页仪表盘与 macOS 完全一致
- 安装脚本可一键部署与卸载，登录后自启

**成功标准**：在 CachyOS + KDE Wayland + fcitx5-rime 上运行 `scripts/install.sh` 后，托盘出现图标，菜单显示与网页一致的今日字数；打字后一分钟内数字更新；重启登录后自动恢复。

## 2. 已确认的事实

1. 本机 Rime 数据位于 `~/.local/share/fcitx5/rime/stats.userdb`，key 为 `statistics/day/YYYYMMDD/...`，value 为 `c=<int> d=0 t=0`，与 macOS 一致，解析层无需改动。
2. `tray-icon 0.19` 在 Linux 走 AppIndicator：**不产生点击事件**、tooltip 为空实现，`set_title` 会设为 AppIndicator label，但 KDE 面板通常不显示该 label。
3. `muda 0.15` 的 `MenuItem::set_text` 在 Linux 可用，可在同步后原地更新菜单文案。
4. `libappindicator-sys` 在运行时动态加载 `libayatana-appindicator3.so.1` 或 `libappindicator3.so.1`；编译期需要 GTK3 开发包（`tray-icon` → `muda` → `gtk`）。
5. 本机 `graphical-session.target` 处于 active，且 systemd 用户环境已导入 `WAYLAND_DISPLAY` / `DISPLAY` / `DBUS_SESSION_BUS_ADDRESS`。
6. Linux（rime 1.17 / fcitx5）的 LevelDB block 使用 Snappy 压缩（type=1），而 macOS Squirrel 为未压缩。解析层需同时支持 `none` 与 Snappy；Snappy 解压自行实现，不新增依赖。

## 3. 方案

采用已选定的「菜单项显示今日字数」：

- macOS：保持菜单栏标题 `⌨ 1,234` 与现有交互不变。
- Linux：面板只显示图标；菜单结构为
  1. `今日 1,234 字`（禁用项，随同步更新）
  2. `打开统计`
  3. `立即同步`
  4. `打开数据目录`
  5. `退出`

被否决的备选：

- 把数字画进图标位图：22px 下只能容纳 3~4 个字符，可读性差，且需要自建像素字体。
- Linux 不做托盘：丢失常驻可见性，与 macOS 体验差距过大。

## 4. 路径解析

新增 `src/paths.rs`，把路径逻辑从 `main.rs` 抽出并做成可单测的纯函数。

### Linux

Rime 目录按顺序探测，取第一个存在 `stats.userdb` 的候选；都不存在时用 fcitx5 默认路径（便于错误信息指向最可能的目录）：

1. `$RIME_COUNTER_RIME_DIR`（显式覆盖，最高优先级）
2. `$XDG_DATA_HOME/fcitx5/rime/stats.userdb`（默认 `~/.local/share/fcitx5/rime/stats.userdb`）
3. `~/.config/ibus/rime/stats.userdb`
4. `~/.config/fcitx/rime/stats.userdb`
5. `~/.config/fcitx5/rime/stats.userdb`

数据目录：`$RIME_COUNTER_DATA_DIR` → `$XDG_DATA_HOME/rime-counter` → `~/.local/share/rime-counter`。

### macOS

保持现状：`$RIME_COUNTER_RIME_DIR` → `~/Library/Rime/stats.userdb`；数据目录 `$RIME_COUNTER_DATA_DIR` → `~/Library/Application Support/rime-counter`。

## 5. 托盘

- 事件循环与同步线程结构不变，仅用 `cfg` 隔离 macOS 专属调用：
  - `set_activation_policy(ActivationPolicy::Accessory)`
  - `with_menu_on_left_click(false)`
  - `with_icon_as_template(true)`
- 打开方式：macOS `open`，Linux `xdg-open`。
- Linux 图标改用蓝色 `#3987e5`（与仪表盘主色一致），避免白色图标在浅色面板上不可见；macOS 继续使用白色模板图标。
- Linux 同步完成后用 `MenuItem::set_text` 更新「今日」项，文案变化时才写入（沿用现有去重思路，避免无意义刷新）。
- Linux 不调用 `set_title`；KDE 不显示 label，调用没有收益。

## 6. 安装与自启

`scripts/install.sh` / `scripts/uninstall.sh` 按 `uname -s` 分支。

Linux 安装：

1. `cargo build --release`
2. 安装到 `${XDG_DATA_HOME:-$HOME/.local/share}/rime-counter/bin/rime-counter`
3. 写 systemd 用户服务 `${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/rime-counter.service`
   - `After=graphical-session.target`、`PartOf=graphical-session.target`
   - `Restart=on-failure`、`RestartSec=5`
   - `WantedBy=graphical-session.target`
4. `systemctl --user daemon-reload`，`enable`，`restart`
5. 通过 `systemctl --user is-active` 等待就绪，并提示 `journalctl --user -u rime-counter`

不依赖 systemd 的环境不在本次范围内，可直接运行二进制。

## 7. 文档

README 增加 Linux 章节：

- 依赖：Arch `gtk3` + 运行时 `libappindicator-gtk3`（或 `libayatana-appindicator`）；Debian/Ubuntu `libgtk-3-dev` + `libayatana-appindicator3-1`
- 路径与环境变量
- 托盘差异说明（数字在菜单里，不在面板上）
- 安装 / 卸载 / 查看日志命令

## 8. 测试

| 层级 | 方式 |
|---|---|
| 路径解析 | 纯函数单测：覆盖 XDG 覆盖、探测顺序、回退默认值 |
| Snappy 解压 | literal / 重叠 copy / 单字节 offset / 截断 / 非法 offset / 长度不符 |
| SSTable | 补充 Snappy 压缩 block 的解析用例（原有用例保持） |
| 现有回归 | `cargo test` 全量通过（51 个） |
| 构建 | Linux `cargo build --release` 无警告 |
| 手动 | 本机安装后核对：托盘菜单字数 == 网页今日字数；打字后 60 秒内更新；`systemctl --user status` 正常；卸载脚本清理干净 |

## 9. 明确不做

- 图标内渲染数字
- GNOME AppIndicator 扩展检测、Wayland 自绘图层
- 非 systemd 发行版的守护进程方案
- Windows 支持
