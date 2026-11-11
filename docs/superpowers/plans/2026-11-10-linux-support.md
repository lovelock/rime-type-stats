# Linux 兼容 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在保持 macOS 行为不变的前提下，让 rime-type-stats 在 Linux（fcitx5-rime 为主）上通过系统托盘与网页仪表盘工作。

**Architecture:** 把平台相关的路径解析抽到纯函数 `paths` 模块；托盘用 `cfg` 隔离 macOS 专属 API，Linux 用 AppIndicator 菜单项显示今日字数；安装脚本按 `uname` 分派到 macOS LaunchAgent 或 Linux systemd 用户服务。解析、存储、同步、HTTP 层不改。

**Tech Stack:** Rust 1.99、`tray-icon 0.19`（Linux 走 libappindicator）、`tao 0.30`、GTK3、systemd 用户服务

**Spec:** `docs/superpowers/specs/2026-11-10-linux-support-design.md`

## Global Constraints

- **macOS 行为不变**：菜单栏标题 `⌨ 1,234`、左键打开网页、右键菜单、LaunchAgent 安装方式全部保留。
- **解析层零改动**：`src/rime/`、`src/store.rs`、`src/sync.rs`、`src/server.rs` 不修改。
- **只读 Rime 数据**：继续按文件名读取 `*.ldb` / `*.log`，不打开 LevelDB、不碰 `LOCK`。
- **仅本地监听**：HTTP 仍绑定 `127.0.0.1`。
- **环境变量覆盖优先**：`RIME_TYPE_STATS_RIME_DIR` / `RIME_TYPE_STATS_DATA_DIR` 语义不变。
- **中文注释与提交信息**：提交信息使用 Conventional Commits 前缀 + 中文正文。
- **零新增依赖**：不新增 crate；Linux 支持靠现有 `tray-icon` / `tao` 的 Linux 实现。

## Review Focus

- `XDG_DATA_HOME` / `RIME_TYPE_STATS_DATA_DIR` 为空字符串时必须回退到默认值，不能生成相对路径或 `rime-type-stats` 在空目录下。
- 多个 Rime 候选目录同时存在时必须按 fcitx5 → ibus → fcitx4 → fcitx5(配置目录) 的固定顺序取第一个，结果可预测。
- 一个候选目录都不存在时必须回退到 fcitx5 默认路径，让 `--dump` 的错误信息指向最可能的位置。
- Rime 目录缺失或读取失败时，托盘不得退出或崩溃，菜单保留上一次成功字数（现有 `SyncState::replace` 语义，需在端到端验证中确认）。
- systemd 服务在图形会话未就绪时启动失败，必须由 `Restart=on-failure` 自动恢复，而不是永久退出。

---

### Task 1: 跨平台路径解析

**Files:**
- Create: `src/paths.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs:1-28`
- Test: `src/paths.rs` 内联测试

**Interfaces:**
- Consumes: 无
- Produces:
  - `rime_counter::paths::home_dir() -> PathBuf`
  - `rime_counter::paths::rime_dir() -> PathBuf` — 当前平台的 Rime 统计库目录
  - `rime_counter::paths::data_dir() -> PathBuf` — 当前平台的数据目录
  - `rime_counter::paths::linux_rime_dir(env: &dyn Fn(&str) -> Option<OsString>, home: &Path, exists: &dyn Fn(&Path) -> bool) -> PathBuf`
  - `rime_counter::paths::linux_data_dir(env: &dyn Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf`
  - `rime_counter::paths::macos_rime_dir(env: &dyn Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf`
  - `rime_counter::paths::macos_data_dir(env: &dyn Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf`

  四个纯函数在所有平台都编译，测试跨平台可跑；`rime_dir` / `data_dir` 用 `cfg` 选择。

- [ ] **Step 1: 创建带失败测试的 `src/paths.rs`**

先只写签名 + `unimplemented!()` 和测试。测试内容：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(v)))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn exists_in(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p| set.iter().any(|candidate| candidate == p)
    }

    #[test]
    fn linux_data_dir_prefers_env_override() {
        let env = env_of(&[
            ("RIME_TYPE_STATS_DATA_DIR", "/custom/data"),
            ("XDG_DATA_HOME", "/xdg"),
        ]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/custom/data")
        );
    }

    #[test]
    fn linux_data_dir_uses_xdg_data_home() {
        let env = env_of(&[("XDG_DATA_HOME", "/xdg")]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/xdg/rime-type-stats")
        );
    }

    #[test]
    fn linux_data_dir_falls_back_to_local_share() {
        let env = env_of(&[]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/home/u/.local/share/rime-type-stats")
        );
    }

    #[test]
    fn empty_env_values_are_ignored() {
        let env = env_of(&[
            ("RIME_TYPE_STATS_DATA_DIR", ""),
            ("XDG_DATA_HOME", ""),
            ("RIME_TYPE_STATS_RIME_DIR", ""),
        ]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/home/u/.local/share/rime-type-stats")
        );
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists_in(&[])),
            PathBuf::from("/home/u/.local/share/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_prefers_env_override() {
        let env = env_of(&[("RIME_TYPE_STATS_RIME_DIR", "/custom/stats.userdb")]);
        let exists = exists_in(&["/home/u/.local/share/fcitx5/rime/stats.userdb"]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/custom/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_picks_first_existing_candidate() {
        let env = env_of(&[]);
        let exists = exists_in(&["/home/u/.config/ibus/rime/stats.userdb"]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/home/u/.config/ibus/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_prefers_fcitx5_when_multiple_exist() {
        let env = env_of(&[]);
        let exists = exists_in(&[
            "/home/u/.local/share/fcitx5/rime/stats.userdb",
            "/home/u/.config/ibus/rime/stats.userdb",
        ]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/home/u/.local/share/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_respects_xdg_data_home() {
        let env = env_of(&[("XDG_DATA_HOME", "/xdg")]);
        let exists = exists_in(&["/xdg/fcitx5/rime/stats.userdb"]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/xdg/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_defaults_to_fcitx5_when_none_exist() {
        let env = env_of(&[]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists_in(&[])),
            PathBuf::from("/home/u/.local/share/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn macos_paths_use_library_dirs() {
        let env = env_of(&[]);
        assert_eq!(
            macos_rime_dir(&env, Path::new("/Users/u")),
            PathBuf::from("/Users/u/Library/Rime/stats.userdb")
        );
        assert_eq!(
            macos_data_dir(&env, Path::new("/Users/u")),
            PathBuf::from("/Users/u/Library/Application Support/rime-type-stats")
        );
    }
}
```

在 `src/lib.rs` 加 `pub mod paths;`，运行测试确认失败。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test paths`
Expected: 编译通过但 9 个测试 panic（`unimplemented!`）。

- [ ] **Step 3: 实现 `src/paths.rs`**

实现要点：

- `home_dir()`：`std::env::var_os("HOME")` → `PathBuf`，缺失/空时 `.`（沿用 `main.rs` 现有行为）。
- `fn env_path(env: &dyn Fn(&str) -> Option<OsString>, key: &str) -> Option<PathBuf>`：空值视为未设置。
- `fn xdg_data_home(env, home) -> PathBuf`：非空 `XDG_DATA_HOME`，否则 `home/.local/share`。
- `linux_data_dir`：`RIME_TYPE_STATS_DATA_DIR` → `xdg_data_home/rime-type-stats`。
- `linux_rime_dir`：`RIME_TYPE_STATS_RIME_DIR` → 依次探测
  1. `xdg_data_home/fcitx5/rime/stats.userdb`
  2. `home/.config/ibus/rime/stats.userdb`
  3. `home/.config/fcitx/rime/stats.userdb`
  4. `home/.config/fcitx5/rime/stats.userdb`

  返回第一个 `exists()` 为真的；都没有则返回候选 1。
- `macos_rime_dir`：`RIME_TYPE_STATS_RIME_DIR` → `home/Library/Rime/stats.userdb`。
- `macos_data_dir`：`RIME_TYPE_STATS_DATA_DIR` → `home/Library/Application Support/rime-type-stats`。
- `rime_dir()` / `data_dir()`：取 `|k| std::env::var_os(k)` 与 `home_dir()`，按 `cfg(target_os)` 调对应纯函数；Linux 的 `exists` 用 `|p: &Path| p.is_dir()`。非 macOS/Linux 平台回退到 Linux 逻辑并加 `#[allow(dead_code)]` 不必要。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test paths`
Expected: 9 passed。

- [ ] **Step 5: 让 `main.rs` 使用新模块**

删除 `main.rs` 里的 `home_dir` / `rime_dir` / `data_dir`，改为：

```rust
use rime_counter::paths::{data_dir, rime_dir};
```

其余逻辑（`--dump`、`--serve-only`、`Store::open`、`sync_once`）不动。运行 `cargo build` 确认无未使用函数或导入警告。

- [ ] **Step 6: 提交**

```bash
git add src/paths.rs src/lib.rs src/main.rs
git commit -m "feat: 增加跨平台路径解析

Linux 自动探测 fcitx5/ibus/fcitx 的 stats.userdb，数据目录遵循 XDG；
macOS 路径行为保持不变。"
```

---

### Task 2: Linux 托盘与图标

**Files:**
- Modify: `src/icons.rs`
- Modify: `src/tray.rs`
- Test: `src/icons.rs` 内联测试

**Interfaces:**
- Consumes: Task 1 的 `paths`（`tray.rs` 不直接用，仅 `main.rs` 用）
- Produces:
  - `rime_counter::icons::today_menu_text(today_chars: u64) -> String` — 形如 `今日 1,234 字`
  - `rime_counter::icons::ICON_RGBA` — macOS 白色模板图标，Linux 蓝色 `#3987e5` 图标
  - `rime_counter::tray::run_app(rime_dir: PathBuf, data_dir: PathBuf, state: SyncState, port: u16) -> !`（签名不变）

- [ ] **Step 1: 写失败测试**

在 `src/icons.rs` 的测试模块追加：

```rust
#[test]
fn formats_today_menu_text() {
    assert_eq!(today_menu_text(0), "今日 0 字");
    assert_eq!(today_menu_text(1234), "今日 1,234 字");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_icon_uses_accent_color() {
    let (_, _, rgba) = ICON_RGBA;
    let px = rgba.chunks_exact(4).find(|p| p[3] > 200).unwrap();
    assert_eq!(&px[..3], &[0x39, 0x87, 0xe5]);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_icon_is_white_template() {
    let (_, _, rgba) = ICON_RGBA;
    let px = rgba.chunks_exact(4).find(|p| p[3] > 200).unwrap();
    assert_eq!(&px[..3], &[255, 255, 255]);
}
```

Run: `cargo test formats_today_menu_text`
Expected: 编译失败，`today_menu_text` 未定义。

- [ ] **Step 2: 实现图标与文案**

- `today_menu_text(n: u64) -> String`：`format!("今日 {} 字", format_thousands(n))`。
- 把 `build_icon()` 改为 `const fn build_icon(rgb: [u8; 3]) -> [u8; ...]`，用 `rgb` 填 `buf[idx..idx+3]`。
- 新增 `const fn icon_rgb() -> [u8; 3]`：macOS `[255, 255, 255]`，其他平台 `[0x39, 0x87, 0xe5]`（用 `#[cfg]` 块返回值）。
- `ICON_RGBA` 改为 `(SIZE, SIZE, &build_icon(icon_rgb()))`。

- [ ] **Step 3: 运行图标测试确认通过**

Run: `cargo test icons`
Expected: 全部通过（含原有 3 个图标/格式化测试）。

- [ ] **Step 4: 改造 `tray.rs`**

目标结构（保持现有事件流与去重逻辑）：

- 导入按平台隔离（避免未使用警告）：
  - `#[cfg(target_os = "macos")] use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};`
  - `#[cfg(target_os = "macos")] use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};`
  - `use crate::icons::ICON_RGBA;`，再按平台分别导入 `menu_bar_title`（macOS）或 `today_menu_text`（Linux）。
- 打开命令：`const OPENER: &str`，macOS `open`、Linux `xdg-open`；`open_in_browser` / `open_data_dir` 都改用它。
- 创建事件循环后，`set_activation_policy(ActivationPolicy::Accessory)` 只在 macOS 调用。
- 菜单构建：
  - Linux 先插入禁用项 `MenuItem::new(today_menu_text(today_chars(&state)), false, None)`，并保存到 `menu_today`；macOS 不插入，改为计算 `initial_title`。
  - 其余四项保持不变。
  - Builder 用两份 `cfg` 构造，避免 Linux 上出现 `unused_mut`：
    - macOS：`TrayIconBuilder::new().with_menu(Box::new(menu)).with_icon(build_icon()).with_tooltip("Rime 打字统计").with_menu_on_left_click(false).with_icon_as_template(true).with_title(&initial_title)`
    - 其他平台：同样的 `with_menu` / `with_icon` / `with_tooltip`，不附加 macOS 专属方法。
    - 最后统一 `let icon = builder.build();`
- `Event::MainEventsCleared`：`TrayIconEvent::receiver().try_recv()` 的轮询只在 macOS 编译；菜单项处理通用。
- `Event::UserEvent(UserEvent::Synced)`：
  - macOS：保留现有 `shown_title` 去重后 `set_title`。
  - Linux：`menu_today` 存在时计算 `today_menu_text(today_chars(&state))`，与 `shown_menu` 不同才 `set_text` 并记录。
- 不调用 `set_title`、`with_title`（Linux 分支），因为 KDE 不显示 AppIndicator label。
- 保留 macOS 关于 `setTitle` 自持重绘循环的注释。

Run: `cargo build`
Expected: Linux 编译通过，无警告。

- [ ] **Step 5: 回归整个测试套件**

Run: `cargo test`
Expected: 全部通过（原有 51 个 + Task 1 的 9 个 + Task 2 的 3 个，允许因平台 cfg 少 1 个）。

- [ ] **Step 6: 提交**

```bash
git add src/icons.rs src/tray.rs
git commit -m "feat: 托盘支持 Linux

Linux 用 AppIndicator 菜单项显示今日字数，图标改为主色蓝以适配浅色面板；
macOS 的标题与模板图标行为不变。"
```

---

### Task 3: 安装脚本与 README

**Files:**
- Modify: `scripts/install.sh`
- Modify: `scripts/uninstall.sh`
- Create: `scripts/install-macos.sh`
- Create: `scripts/uninstall-macos.sh`
- Create: `scripts/install-linux.sh`
- Create: `scripts/uninstall-linux.sh`
- Modify: `README.md`

**Interfaces:**
- Consumes: `target/release/rime-type-stats`
- Produces: `scripts/install.sh` / `scripts/uninstall.sh` 在 macOS 与 Linux 上均可用

- [ ] **Step 1: 拆出 macOS 脚本**

- 把现有 `scripts/install.sh` 全部内容原样移到 `scripts/install-macos.sh`（仅改第一行注释为 macOS）。
- 把现有 `scripts/uninstall.sh` 全部内容原样移到 `scripts/uninstall-macos.sh`。
- 两个新文件 `chmod +x`。

- [ ] **Step 2: 写分派脚本**

`scripts/install.sh`：

```bash
#!/usr/bin/env bash
# 按平台分派安装脚本。
set -euo pipefail

case "$(uname -s)" in
  Darwin) exec "$(dirname "$0")/install-macos.sh" ;;
  Linux) exec "$(dirname "$0")/install-linux.sh" ;;
  *)
    echo "不支持的系统：$(uname -s)" >&2
    exit 1
    ;;
esac
```

`scripts/uninstall.sh` 同结构，分派到 `uninstall-macos.sh` / `uninstall-linux.sh`。两者都 `chmod +x`。

- [ ] **Step 3: 写 Linux 安装脚本**

`scripts/install-linux.sh`：

```bash
#!/usr/bin/env bash
# 编译、安装并注册 rime-type-stats 为 systemd 用户服务。
set -euo pipefail

APP_NAME="rime-type-stats"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
INSTALL_DIR="$DATA_HOME/$APP_NAME/bin"
BIN="$INSTALL_DIR/$APP_NAME"
UNIT_DIR="$CONFIG_HOME/systemd/user"
UNIT="$UNIT_DIR/$APP_NAME.service"

cd "$(dirname "$0")/.."

echo "==> 检查编译依赖"
if ! pkg-config --exists gtk+-3.0; then
  echo "缺少 GTK3 开发包：Arch 用 sudo pacman -S gtk3；Debian/Ubuntu 用 sudo apt install libgtk-3-dev" >&2
  exit 1
fi

echo "==> 编译 release 版本"
cargo build --release

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
rm -f "$BIN"
install -m 755 "target/release/$APP_NAME" "$BIN"

echo "==> 写入 systemd 用户服务：$UNIT"
mkdir -p "$UNIT_DIR"
cat > "$UNIT" <<UNIT_EOF
[Unit]
Description=Rime 打字统计
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=$BIN
Restart=on-failure
RestartSec=5

[Install]
WantedBy=graphical-session.target
UNIT_EOF

echo "==> 启动服务"
systemctl --user daemon-reload
systemctl --user enable "$APP_NAME.service"
systemctl --user restart "$APP_NAME.service"

echo "==> 等待服务就绪"
for _ in $(seq 1 20); do
  if systemctl --user is-active --quiet "$APP_NAME.service"; then
    echo "==> 已启动。托盘菜单第一项显示今日字数。"
    echo "    查看日志：journalctl --user -u $APP_NAME -f"
    echo "    卸载：scripts/uninstall.sh"
    exit 0
  fi
  sleep 0.5
done

echo "!! 服务未能启动，请查看：journalctl --user -u $APP_NAME -n 50" >&2
exit 1
```

`scripts/uninstall-linux.sh`：

```bash
#!/usr/bin/env bash
# 停止并移除 rime-type-stats 的 systemd 用户服务与二进制，保留统计数据。
set -euo pipefail

APP_NAME="rime-type-stats"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
UNIT="$CONFIG_HOME/systemd/user/$APP_NAME.service"
INSTALL_DIR="$DATA_HOME/$APP_NAME/bin"

if [[ -f "$UNIT" ]]; then
  echo "==> 停止并移除 systemd 用户服务"
  systemctl --user disable --now "$APP_NAME.service" 2>/dev/null || true
  rm -f "$UNIT"
  systemctl --user daemon-reload
fi

echo "==> 删除二进制：$INSTALL_DIR"
rm -rf "$INSTALL_DIR"

echo "==> 数据（events.jsonl）已保留：$DATA_HOME/$APP_NAME"
```

两个脚本 `chmod +x`。运行 `bash -n scripts/*.sh` 确认语法。

- [ ] **Step 4: 更新 README**

必改内容：

- 开头一句：同时支持 macOS（Squirrel）与 Linux（fcitx5-rime / ibus-rime）。
- 「原理」：Rime 路径改为 macOS `~/Library/Rime/stats.userdb` / Linux `~/.local/share/fcitx5/rime/stats.userdb`。
- 「安装」拆成 macOS / Linux 两小节：
  - macOS 保持现有说明与 LaunchAgent 细节。
  - Linux 依赖：Arch `gtk3`（编译）+ `libappindicator-gtk3` 或 `libayatana-appindicator`（运行）；Debian/Ubuntu `libgtk-3-dev`（编译）+ `libayatana-appindicator3-1`（运行）。
  - Linux 安装后：托盘菜单第一项显示今日字数，点开菜单可打开仪表盘；日志用 `journalctl --user -u rime-type-stats -f`。
- 「卸载」补充 Linux 命令（`scripts/uninstall.sh`），数据目录 `~/.local/share/rime-type-stats` 保留。
- 「开发」环境变量说明补 Linux 默认值：Rime 目录自动探测、数据目录 `~/.local/share/rime-type-stats`。
- 「架构」表加 `src/paths.rs`（跨平台路径解析）。
- 新增「Linux 托盘差异」一句话：AppIndicator 不支持 macOS 式标题，字数放在菜单第一项。

- [ ] **Step 5: 验证脚本语法**

Run: `bash -n scripts/install.sh scripts/uninstall.sh scripts/install-macos.sh scripts/uninstall-macos.sh scripts/install-linux.sh scripts/uninstall-linux.sh`
Expected: 无输出、退出码 0。

- [ ] **Step 6: 提交**

```bash
git add scripts README.md
git commit -m "feat: Linux 安装脚本与文档

install.sh/uninstall.sh 按平台分派；Linux 使用 systemd 用户服务，
README 补充依赖、路径与托盘差异说明。"
```

---

### Task 4: 端到端验证

**Files:**
- 无代码改动（只验证）

**Interfaces:**
- Consumes: Task 1-3 的全部产物
- Produces: 可交付的 Linux 版本

- [ ] **Step 1: 全量测试与构建**

Run: `cargo test && cargo build --release`
Expected: 全部测试通过；release 构建无警告。

- [ ] **Step 2: 验证真实数据解析**

Run: `cargo run -- --dump`
Expected: 输出「共 N 天有记录」且 N > 0，包含 `2026-10-31` 之后的数据；说明 Linux 路径探测生效。

- [ ] **Step 3: 验证 HTTP 接口**

Run:

```bash
cargo run -- --serve-only &
sleep 3
curl -s http://127.0.0.1:8787/api/days | head -c 300
kill %1
```

Expected: JSON 含 `"sync"` 与 `"days"`，`days` 非空。若 8787 被占用，用启动日志打印的端口。

- [ ] **Step 4: 验证 Rime 目录缺失时服务不崩溃**

Run:

```bash
RIME_TYPE_STATS_RIME_DIR=/nonexistent/rime cargo run -- --serve-only &
sleep 3
curl -s http://127.0.0.1:8787/api/days
kill %1
```

Expected: 进程保持运行，JSON 的 `sync.error` 非空、`days` 为空；不会 panic 或退出。

- [ ] **Step 5: 安装并检查服务**

Run: `scripts/install.sh`，然后 `systemctl --user is-active rime-type-stats`
Expected: `active`；`journalctl --user -u rime-type-stats -n 20` 显示 `rime-type-stats 已启动：http://127.0.0.1:<port>/`。

- [ ] **Step 6: 验证崩溃自动重启**

Run:

```bash
kill "$(systemctl --user show -p MainPID --value rime-type-stats)"
sleep 7
systemctl --user is-active rime-type-stats
```

Expected: 输出 `active`，说明 `Restart=on-failure` 生效。

- [ ] **Step 7: 手动确认托盘**

人工确认：托盘出现蓝色键盘图标，菜单第一项为「今日 N 字」，N 与网页今日字数一致；打字后 60 秒内更新。

- [ ] **Step 8: 验证卸载后数据保留**

Run: `scripts/uninstall.sh && ls "$HOME/.local/share/rime-type-stats/events.jsonl"`
Expected: 服务与二进制被移除，`events.jsonl` 仍存在。验证完成后重跑 `scripts/install.sh` 恢复常驻。

- [ ] **Step 9: 提交验证记录（可选）**

如无需提交文件，此步跳过；若修正了验证中发现的缺陷，按对应 Task 的提交规范补充提交。
