# 上游同步与 Linux 打包 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Linux 支持 rebase 到重写后的 `origin/main`，统一改名为 `rime-type-stats`，并在 CI 中构建/发布 x86_64 的 deb、rpm、Arch 包。

**Architecture:** 先用 `git rebase --onto` 把本地提交接到上游新历史，按上游命名解决冲突；再补一个纯改名提交；最后用 nfpm 一份配置生成三种包，由 `ci.yml` 做测试与打包预演，`release.yml` 在 `v*` 标签时把产物加入同一个 GitHub Release。

**Tech Stack:** Rust 1.99、Git、GitHub Actions、nfpm v2.48.0、systemd 用户单元

**Spec:** `docs/superpowers/specs/2026-11-11-upstream-sync-linux-packaging-design.md`

## Global Constraints

- **macOS 行为与 DMG 流程不变**：现有 `ci.yml` 的 macOS job、`release.yml` 的 `build-dmg` 与 `publish` 逻辑不得删改，只做增量。
- **只出 x86_64**：nfpm 的 `arch: amd64`，不构建 aarch64。
- **许可证 MIT**：新增 `LICENSE`，`Cargo.toml` 增加 `license = "MIT"`。
- **包内容固定**：`/usr/bin/rime-type-stats` + `/usr/lib/systemd/user/rime-type-stats.service` + `/usr/share/licenses/rime-type-stats/LICENSE`。
- **不自动 enable 用户服务**：包只安装单元。
- **nfpm 固定 v2.48.0**：下载后必须用官方 `checksums.txt` 校验 sha256。
- **版本规范化**：传给 nfpm 的版本把 `-` 替换为 `.`（`0.1.0-beta.1` → `0.1.0.beta.1`），避免 Arch `pkgver` 异常。
- **零新增 Rust 依赖**。
- **中文注释与提交信息**：Conventional Commits 前缀 + 中文正文。
- **每次提交前**：`cargo fmt --all --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked` 全绿。

## Review Focus

- rebase 不能丢掉上游对 `main.rs`、`README.md`、`scripts/`、`.github/` 的改动；冲突解决后必须逐文件对照 `origin/main`。
- 任何 `rime-counter` / `RIME_COUNTER` 残留都会导致数据目录、systemd 单元或文档与实际不符；rebase 后必须 `grep -R` 清零。
- deb 的 `libgtk-3-0 | libgtk-3-0t64` 是 Debian 依赖替代语法，nfpm 必须原样写入 `Depends`。
- Arch 包必须包含 `/usr/lib/systemd/user/rime-type-stats.service`，否则安装后无法启用。
- `publish` 只能上传 DMG、deb、rpm、Arch 四类资产，不能把渲染后的 `nfpm.yaml` 带上去。
- 预发布标签（含 `-`）必须经过版本规范化后仍能生成三种包。
- `package-linux` 预演 job 必须真正运行 nfpm 并校验内容，而不是只做 `cargo build`。

---

### Task 1: 同步上游历史

**Files:**
- Modify: `Cargo.toml`、`Cargo.lock`、`src/main.rs`、`README.md`、`scripts/install.sh`、`scripts/uninstall.sh`（rebase 冲突）
- Create during conflict: `scripts/install-macos.sh`、`scripts/uninstall-macos.sh`（取上游内容）
- Keep: `scripts/install-linux.sh`、`scripts/uninstall-linux.sh`（旧名，Task 2 改名）

**Interfaces:**
- Consumes: `origin/main`（`4ed8cd0`）、旧基线 `4f0d7c5`
- Produces: 线性历史 `origin/main..HEAD`，crate 名为 `rime_type_stats`，`cargo test --locked` 通过

- [ ] **Step 1: 确认工作区干净并执行 rebase**

```bash
git status --short
git rebase --onto origin/main 4f0d7c5 HEAD
```

Expected: 在 `1332c45`（路径解析）和 `cd064ae`（脚本/README）处出现冲突。

- [ ] **Step 2: 解决 `Cargo.toml` 冲突**

最终内容必须包含上游的包名与本地关闭 `libxdo` 的改动：

```toml
[package]
name = "rime-type-stats"
version = "0.1.0"
edition = "2021"

[lib]
name = "rime_type_stats"
path = "src/lib.rs"

[[bin]]
name = "rime-type-stats"
path = "src/main.rs"

[dependencies]
chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tray-icon = { version = "0.19", default-features = false }
tao = "0.30"
```

- [ ] **Step 3: 解决 `Cargo.lock` 冲突**

先取上游版本的 lock（rebase 中 `--ours` 指上游基线）：

```bash
git checkout --ours Cargo.lock
```

真正更新 lock 放到 Step 7 统一执行（此时所有冲突已解决，`cargo build` 才能跑通）。

- [ ] **Step 4: 解决 `src/main.rs` 冲突**

保留上游的新名称与 `--dump` / `--serve-only` 逻辑，删除本地路径函数，改为：

```rust
use rime_type_stats::paths::{data_dir, rime_dir};
use rime_type_stats::server::{bind_first_available, serve};
use rime_type_stats::store::Store;
use rime_type_stats::sync::{sync_once, to_iso_day, SyncState};
use rime_type_stats::tray;
```

并确保 `print_days` 的类型路径为 `rime_type_stats::rime::parser::DayStats`。

- [ ] **Step 5: 解决脚本冲突**

冲突时：

```bash
git show origin/main:scripts/install.sh > scripts/install-macos.sh
git show origin/main:scripts/uninstall.sh > scripts/uninstall-macos.sh
git checkout --theirs scripts/install.sh scripts/uninstall.sh
chmod +x scripts/install.sh scripts/uninstall.sh scripts/install-macos.sh scripts/uninstall-macos.sh
```

- `scripts/install.sh` / `uninstall.sh` 保持本地分派脚本。
- `scripts/install-macos.sh` / `uninstall-macos.sh` 使用上游最新内容（已包含 `rime-type-stats`、`com.lovelock.rime-type-stats`）。
- `scripts/install-linux.sh` / `uninstall-linux.sh` 暂时保留旧名，Task 2 统一修改。

- [ ] **Step 6: 解决 `README.md` 冲突**

以上游最新 README 为底，合并本地的 Linux 章节；本步骤只要求文件能提交，名称统一留到 Task 2。

- [ ] **Step 7: 更新 lock、继续 rebase 并跑测试**

```bash
cargo build
git add Cargo.lock
git add -A
git rebase --continue
cargo test --locked
```

Expected: `Cargo.lock` 中包名为 `rime-type-stats` 且不含 `libxdo`；8 个提交（7 个 Linux 提交 + 设计文档）线性落在 `origin/main` 上，测试全部通过。

- [ ] **Step 8: 确认上游专属文件没有被改动**

```bash
git diff --stat origin/main -- .github assets scripts/bundle.sh scripts/gen_icns.py
```

Expected: 无输出（本次 rebase 不应触碰这些文件）。

- [ ] **Step 9: 提交 rebase 结果（如 rebase 未自动完成）**

rebase 正常结束时无需额外提交；确认 `git log --oneline origin/main..HEAD` 有 8 条记录即可。

---

### Task 2: 统一改名为 rime-type-stats

**Files:**
- Modify: `src/paths.rs`、`src/tray.rs`、`scripts/install-linux.sh`、`scripts/uninstall-linux.sh`、`README.md`
- Modify: `docs/superpowers/specs/2026-11-10-linux-support-design.md`、`docs/superpowers/plans/2026-11-10-linux-support.md`

**Interfaces:**
- Consumes: Task 1 的 `rime_type_stats` crate 名
- Produces: `paths::rime_dir()` / `data_dir()` 使用 `RIME_TYPE_STATS_*` 与 `rime-type-stats`；`rime-type-stats.service`

- [ ] **Step 1: 先改测试（RED）**

把 `src/paths.rs` 测试中的旧名替换：

- `RIME_COUNTER_RIME_DIR` → `RIME_TYPE_STATS_RIME_DIR`
- `RIME_COUNTER_DATA_DIR` → `RIME_TYPE_STATS_DATA_DIR`
- `/custom/data` 等期望值中的 `rime-counter` → `rime-type-stats`

Run: `cargo test paths`
Expected: FAIL（实现仍返回旧路径）。

- [ ] **Step 2: 改实现（GREEN）**

`src/paths.rs` 中：

- 环境变量改为 `RIME_TYPE_STATS_RIME_DIR` / `RIME_TYPE_STATS_DATA_DIR`
- 数据目录默认值改为 `rime-type-stats`
- 文档注释同步更新

Run: `cargo test paths`
Expected: 10 passed。

- [ ] **Step 3: 改托盘测试临时目录**

`src/tray.rs` 测试里的 `rime-counter-tray-sync-` 改为 `rime-type-stats-tray-sync-`（无断言影响）。

- [ ] **Step 4: 改 Linux 脚本**

`scripts/install-linux.sh` 与 `uninstall-linux.sh`：

- `APP_NAME="rime-type-stats"`
- systemd 单元 `rime-type-stats.service`
- 提示文案与数据路径统一为 `rime-type-stats`
- 保留 `ExecStart="$BIN"` 的引号

- [ ] **Step 5: 改文档**

用 `grep -R` 找出旧名并替换：

```bash
grep -RIn 'rime-counter\|RIME_COUNTER' --exclude-dir=target --exclude-dir=.git .
```

范围包括 README、`docs/superpowers/specs/2026-11-10-linux-support-design.md`、`docs/superpowers/plans/2026-11-10-linux-support.md`。历史提交里的旧名不需要改。

- [ ] **Step 6: 校验无旧名残留**

Run:

```bash
! grep -RIn 'rime-counter\|RIME_COUNTER' --exclude-dir=target --exclude-dir=.git .
cargo test --locked
```

Expected: grep 无输出；测试全绿。

- [ ] **Step 7: 提交**

```bash
git add -A
git commit -m "refactor: 跟随上游更名为 rime-type-stats

环境变量、数据目录、systemd 单元、安装脚本与文档统一使用新名称。"
```

---

### Task 3: 打包配置、LICENSE 与 README

**Files:**
- Create: `LICENSE`
- Create: `packaging/nfpm.yaml`
- Create: `packaging/rime-type-stats.service`
- Modify: `Cargo.toml`
- Modify: `README.md`
- Test: 本地 nfpm v2.48.0 产物检查

**Interfaces:**
- Consumes: `target/release/rime-type-stats`
- Produces: `dist/*.deb`、`dist/*.rpm`、`dist/*.pkg.tar.zst`

- [ ] **Step 1: 新增 MIT LICENSE**

```text
MIT License

Copyright (c) 2026 rime-type-stats contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 2: 补 `Cargo.toml` 元数据**

在 `edition` 后加入：

```toml
description = "统计 Rime 每日打字字数，提供系统托盘与本地仪表盘"
license = "MIT"
repository = "https://github.com/lovelock/rime-type-stats"
```

- [ ] **Step 3: 新增 systemd 用户单元**

`packaging/rime-type-stats.service`：

```ini
[Unit]
Description=Rime typing statistics
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=/usr/bin/rime-type-stats
Restart=on-failure
RestartSec=5

[Install]
WantedBy=graphical-session.target
```

- [ ] **Step 4: 新增 nfpm 配置**

`packaging/nfpm.yaml`：

```yaml
name: rime-type-stats
arch: amd64
platform: linux
version: __VERSION__
release: "1"
section: utils
priority: optional
maintainer: lovelock <lovelock@users.noreply.github.com>
description: |
  统计 Rime 每日打字字数，提供系统托盘与本地仪表盘。
vendor: lovelock
homepage: https://github.com/lovelock/rime-type-stats
license: MIT
contents:
  - src: target/release/rime-type-stats
    dst: /usr/bin/rime-type-stats
  - src: packaging/rime-type-stats.service
    dst: /usr/lib/systemd/user/rime-type-stats.service
  - src: LICENSE
    dst: /usr/share/licenses/rime-type-stats/LICENSE
overrides:
  deb:
    depends:
      - libgtk-3-0 | libgtk-3-0t64
      - libayatana-appindicator3-1
  rpm:
    depends:
      - gtk3
      - libayatana-appindicator-gtk3
  archlinux:
    depends:
      - gtk3
      - libayatana-appindicator
```

- [ ] **Step 5: 本地生成三种包并检查**

```bash
cargo build --release --locked
mkdir -p /tmp/opencode/nfpm-bin dist
cd /tmp/opencode/nfpm-bin
curl -fsSL -o nfpm.tar.gz https://github.com/goreleaser/nfpm/releases/download/v2.48.0/nfpm_2.48.0_Linux_x86_64.tar.gz
tar xzf nfpm.tar.gz nfpm
cd - >/dev/null
sed 's|__VERSION__|0.1.0|g' packaging/nfpm.yaml > /tmp/opencode/nfpm-rendered.yaml
for p in deb rpm archlinux; do
  /tmp/opencode/nfpm-bin/nfpm package -p "$p" -f /tmp/opencode/nfpm-rendered.yaml -t dist/
done
ls -l dist
```

Expected: 三个文件：`rime-type-stats_0.1.0-1_amd64.deb`、`rime-type-stats-0.1.0-1.x86_64.rpm`、`rime-type-stats-0.1.0-1-x86_64.pkg.tar.zst`。

检查内容：

```bash
ar p dist/*.deb control.tar.gz | tar xzO ./control | grep -E '^(Package|Version|Architecture|Depends):'
zstd -dc dist/*.pkg.tar.zst | tar -tf - | grep -E 'usr/bin/rime-type-stats|systemd/user/rime-type-stats.service'
```

Expected: `Depends` 含 `libgtk-3-0 | libgtk-3-0t64`；Arch 包含二进制与 unit。

- [ ] **Step 6: 更新 README**

在 Linux 章节增加「Linux 安装包」：

```markdown
### Linux 安装包

从 Releases 下载对应格式（x86_64）：

- Debian / Ubuntu：`sudo apt install ./rime-type-stats_*.deb`
- Fedora / RHEL：`sudo dnf install ./rime-type-stats-*.rpm`（RHEL 9 需启用 EPEL）
- Arch：`sudo pacman -U ./rime-type-stats-*.pkg.tar.zst`

然后启用用户服务：

```bash
systemctl --user enable --now rime-type-stats
```

运行时依赖由包管理器处理。安装包与源码脚本二选一；如果之前用过
`scripts/install.sh`，先执行 `scripts/uninstall.sh` 再安装包。
```

- [ ] **Step 7: 提交**

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
git add LICENSE Cargo.toml packaging README.md
git commit -m "build: 增加 deb/rpm/Arch 打包配置

使用 nfpm 统一生成三种 x86_64 包，安装二进制、systemd 用户单元与 MIT 许可证。"
```

---

### Task 4: CI 测试与发布

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: `packaging/nfpm.yaml`、`LICENSE`
- Produces: `ci.yml` 的 `test-linux` / `package-linux`；`release.yml` 的 `build-linux` 与更新后的 `publish`

- [ ] **Step 1: `ci.yml` 增加 Linux 测试 job**

在现有 `test` job 后追加：

```yaml
  test-linux:
    name: fmt / clippy / test (x86_64 Linux)
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7

      - name: 安装编译依赖
        run: |
          sudo apt-get update
          sudo apt-get install -y libgtk-3-dev pkg-config

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - uses: Swatinem/rust-cache@v2

      - name: cargo fmt --check
        run: cargo fmt --all --check

      - name: cargo clippy
        run: cargo clippy --all-targets --locked -- -D warnings

      - name: cargo test
        run: cargo test --locked

      - name: cargo build (release)
        run: cargo build --release --locked
```

- [ ] **Step 2: `ci.yml` 增加打包预演 job**

追加：

```yaml
  package-linux:
    name: packages (x86_64 dry run)
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7

      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: 安装编译与打包工具
        run: |
          sudo apt-get update
          sudo apt-get install -y libgtk-3-dev pkg-config rpm zstd

      - name: 编译
        run: cargo build --release --locked

      - name: 安装 nfpm
        env:
          NFPM_VERSION: v2.48.0
        run: |
          version="${NFPM_VERSION#v}"
          curl -fsSL -o /tmp/nfpm.tar.gz \
            "https://github.com/goreleaser/nfpm/releases/download/${NFPM_VERSION}/nfpm_${version}_Linux_x86_64.tar.gz"
          curl -fsSL -o /tmp/nfpm-checksums.txt \
            "https://github.com/goreleaser/nfpm/releases/download/${NFPM_VERSION}/checksums.txt"
          (cd /tmp && grep "nfpm_${version}_Linux_x86_64.tar.gz$" nfpm-checksums.txt | sha256sum -c -)
          tar -xzf /tmp/nfpm.tar.gz -C /tmp nfpm
          /tmp/nfpm --version

      - name: 生成包
        env:
          VERSION: 0.0.0.dev
        run: |
          mkdir -p dist
          sed "s|__VERSION__|${VERSION}|g" packaging/nfpm.yaml > "$RUNNER_TEMP/nfpm.yaml"
          for packager in deb rpm archlinux; do
            /tmp/nfpm package -p "$packager" -f "$RUNNER_TEMP/nfpm.yaml" -t dist/
          done
          ls -l dist

      - name: 校验产物
        run: |
          shopt -s nullglob
          debs=(dist/*.deb); rpms=(dist/*.rpm); arch=(dist/*.pkg.tar.zst)
          test ${#debs[@]} -eq 1 || { echo "deb 数量异常" >&2; exit 1; }
          test ${#rpms[@]} -eq 1 || { echo "rpm 数量异常" >&2; exit 1; }
          test ${#arch[@]} -eq 1 || { echo "arch 数量异常" >&2; exit 1; }
          dpkg-deb -c "${debs[0]}" | grep -q 'usr/bin/rime-type-stats'
          rpm -qp --requires "${rpms[0]}" | grep -qx gtk3
          zstd -dc "${arch[0]}" | tar -tf - | grep -q 'usr/lib/systemd/user/rime-type-stats.service'

      - uses: actions/upload-artifact@v7
        with:
          name: linux-x86_64-0.0.0.dev
          path: |
            dist/*.deb
            dist/*.rpm
            dist/*.pkg.tar.zst
          if-no-files-found: error
          overwrite: true
          retention-days: 7
```

- [ ] **Step 3: `release.yml` 增加 `build-linux` job**

在 `build-dmg` 后、`publish` 前追加一个 job，步骤与 `package-linux` 类似，差异是：

- 版本计算沿用 `build-dmg` 的逻辑，并额外输出 `nfpm_version`；
- 运行 `cargo test --locked`；
- 用 `nfpm_version` 渲染配置；
- artifact 名为 `linux-x86_64-${{ steps.version.outputs.version }}`。

版本步骤：

```yaml
      - name: 计算版本号
        id: version
        env:
          EVENT_NAME: ${{ github.event_name }}
          INPUT_VERSION: ${{ inputs.version }}
          REF_NAME: ${{ github.ref_name }}
        run: |
          if [ "$EVENT_NAME" = "workflow_dispatch" ]; then
            version="$INPUT_VERSION"
          else
            version="${REF_NAME#v}"
          fi
          case "$version" in
            ""|*[!0-9A-Za-z.+-]*)
              echo "!! 非法版本号：$version" >&2
              exit 1
              ;;
          esac
          echo "version=$version" >> "$GITHUB_OUTPUT"
          echo "nfpm_version=${version//-/.}" >> "$GITHUB_OUTPUT"
          echo "版本号：$version（nfpm：${version//-/.}）"
```

- [ ] **Step 4: 修改 `publish`**

- `needs: [build-dmg, build-linux]`
- 在下载 DMG 的步骤后增加：

```yaml
      - uses: actions/download-artifact@v8
        with:
          name: linux-x86_64-${{ steps.version.outputs.version }}
          path: dist
```

- 把 `files=(dist/*.dmg)` 改为：

```bash
files=(dist/*.dmg dist/*.deb dist/*.rpm dist/*.pkg.tar.zst)
```

并保留 `test ${#files[@]} -gt 0` 的校验。

- [ ] **Step 5: 校验 workflow 语法**

```bash
python3 - <<'PY'
import yaml
for path in [".github/workflows/ci.yml", ".github/workflows/release.yml"]:
    with open(path) as f:
        yaml.safe_load(f)
    print(path, "ok")
PY
```

如果本机有 `actionlint`，再运行 `actionlint`。

Expected: 两个文件均解析成功，无 actionlint 错误。

- [ ] **Step 6: 提交**

```bash
git add .github/workflows/ci.yml .github/workflows/release.yml
git commit -m "ci: 增加 Linux 测试与 deb/rpm/Arch 打包发布

PR/push 上做打包预演；推 v* 标签时把三类包与 DMG 一起发布。"
```

---

### Task 5: 端到端验证

**Files:** 无（只验证，发现问题按对应 Task 修复）

**Interfaces:**
- Consumes: Task 1-4 的全部产物
- Produces: 本机可用的 Arch 包与 systemd 服务

- [ ] **Step 1: 全量检查**

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

Expected: 全绿。

- [ ] **Step 2: 生成稳定版与预发布版三种包**

用 Task 3 的 nfpm 命令分别构建 `0.1.0` 与 `0.1.0.beta.1`，确认：

- 稳定版文件名符合预期；
- 预发布版不会出现 `pkgver` 丢字段；
- 三个包都能生成。

- [ ] **Step 3: 运行 Arch 包内的二进制**

```bash
rm -rf /tmp/opencode/arch-pkg && mkdir -p /tmp/opencode/arch-pkg
zstd -dc dist/*.pkg.tar.zst | tar -xf - -C /tmp/opencode/arch-pkg
/tmp/opencode/arch-pkg/usr/bin/rime-type-stats --dump | head -5
```

Expected: 输出真实天数与今日字数。

- [ ] **Step 4: 检查 deb 元数据**

```bash
ar p dist/*.deb control.tar.gz | tar xzO ./control | grep -E '^(Package|Version|Architecture|Depends):'
```

Expected: `Depends: libgtk-3-0 | libgtk-3-0t64, libayatana-appindicator3-1`。

- [ ] **Step 5: 迁移旧服务并安装 Arch 包**

```bash
systemctl --user disable --now rime-counter.service 2>/dev/null || true
rm -f "$HOME/.config/systemd/user/rime-counter.service"
systemctl --user daemon-reload
rm -rf "$HOME/.local/share/rime-counter/bin"
sudo pacman -U --noconfirm dist/*.pkg.tar.zst
systemctl --user enable --now rime-type-stats.service
sleep 2
systemctl --user is-active rime-type-stats.service
```

Expected: `active`。

- [ ] **Step 6: 确认托盘菜单**

通过 DBus 读取菜单，确认首项为今日字数且其余菜单项存在（同上一轮验证方式）。人工确认托盘图标可见。

- [ ] **Step 7: 提交验证中发现的修复（如有）**

按对应 Task 的提交规范补充提交；无需提交验证记录。
