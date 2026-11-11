# 上游同步与 Linux 打包设计

日期：2026-11-11
状态：待确认

## 1. 目标

1. 把本地 7 个 Linux 支持提交同步到上游重写后的 `origin/main`，统一采用新名称 `rime-type-stats`。
2. 在 GitHub Actions 中增加 Linux 测试，并在推 `v*` 标签时构建 x86_64 的 `.deb`、`.rpm`、Arch `.pkg.tar.zst`，与 macOS DMG 一起发布到同一个 Release。
3. 保持 macOS 行为与现有 DMG 流程不变。

**成功标准**：`git rebase` 后 Linux 与 macOS 两套平台代码共存；`cargo test` / clippy 全绿；本地用 nfpm 能产出三种包且内容正确；CI 的 Linux job 与 release job 语法有效，标签发布时三类包出现在 Release 资产中。

## 2. 已确认的事实

1. `origin/main` 被 force-push 重写：项目更名为 `rime-type-stats`，crate `rime_type_stats`，二进制 `rime-type-stats`，环境变量 `RIME_TYPE_STATS_*`，数据目录 `rime-type-stats`。本地旧历史（`4f0d7c5`）与上游**没有共同祖先**，普通 merge 不可行。
2. 上游已有 `.github/workflows/ci.yml`（macOS arm64 的 fmt/clippy/test）和 `release.yml`（推 `v*` 构建 DMG 并发布），以及 `scripts/bundle.sh`、`assets/AppIcon.icns`。
3. 本地待迁移提交为 `c80fd65..65ef47a`，新增 `paths.rs`、`snappy.rs`、`install-linux.sh` 等，并修改了 `main.rs`、`tray.rs`、`icons.rs`、`Cargo.toml`、`README`、安装脚本。
4. nfpm 最新版本为 **v2.48.0**，已在本地用该版本实测：
   - 能产 `rime-type-stats_0.1.0-1_amd64.deb`、`rime-type-stats-0.1.0-1.x86_64.rpm`、`rime-type-stats-0.1.0-1-x86_64.pkg.tar.zst`；
   - `src` 路径相对**运行目录**（仓库根）解析；
   - 预发布版本（含 `-`）在 Arch 包的 `pkgver` 上行为不一致，需要规避。
5. 仓库目前没有 `LICENSE` 文件，`Cargo.toml` 也没有 description/license/repository 元数据。
6. 上游 Release 资产命名：`nfpm_2.48.0_Linux_x86_64.tar.gz` + `checksums.txt`。

## 3. 方案

### 3.1 上游合并

被否决的备选：

- `git merge --allow-unrelated-histories`：会为所有文件制造冲突，且历史里保留两套重复提交，不可取。
- 从 `origin/main` 新建分支后手工覆盖文件：会丢掉上游对 `main.rs`、`README`、脚本的改动，容易漏改。

采用 **rebase --onto**：

```bash
git rebase --onto origin/main 4f0d7c5 HEAD
```

- 冲突以上游为准，保留 Linux 功能。
- rebase 完成后补一个提交，把新增文件里的旧名统一改为新名：
  - `RIME_COUNTER_RIME_DIR` / `RIME_COUNTER_DATA_DIR` → `RIME_TYPE_STATS_*`
  - 数据目录、systemd 单元、二进制名、日志与文档中的 `rime-counter` → `rime-type-stats`
  - `rime_counter::` → `rime_type_stats::`
- 用 `grep -R` 确认无残留旧名（`docs/` 下历史文档也一并更新）。

### 3.2 CI：Linux 测试与打包预演

在 `ci.yml` 增加 `test-linux` job：

- `runs-on: ubuntu-24.04`
- 安装 `libgtk-3-dev`、`pkg-config`（`libappindicator` 仅在运行时 dlopen，不需要开发包）
- `cargo fmt --all --check`
- `cargo clippy --all-targets --locked -- -D warnings`
- `cargo test --locked`
- `cargo build --release --locked`

同时增加 `package-linux` job，在每次 push / PR 上做**打包预演**（不发布）：

- 用固定版本 `0.0.0.dev` 生成三种包；
- 执行与 release 相同的 nfpm 步骤和产物校验；
- 上传 artifact，保留 7 天。

macOS arm64 的现有 job 保持不变。

### 3.3 CI：Linux 打包与发布

在 `release.yml` 增加 `build-linux` job（`ubuntu-24.04`，x86_64）：

1. 计算版本号（沿用现有 `v*` 去前缀与字符校验逻辑）。
2. 额外生成 **nfpm 版本**：把 `-` 替换为 `.`，规避 nfpm 在 Arch 预发布版本上的不一致（例：`0.1.0-beta.1` → `0.1.0.beta.1`）。稳定版本不受影响。
3. `cargo test --locked` + `cargo build --release --locked`。
4. 下载并校验 nfpm v2.48.0：
   - `nfpm_2.48.0_Linux_x86_64.tar.gz`
   - 与官方 `checksums.txt` 做 `sha256sum -c`
5. 用 `sed` 把 `packaging/nfpm.yaml` 的 `__VERSION__` 替换为 nfpm 版本，生成到 `$RUNNER_TEMP/nfpm.yaml`（不放进 `dist/`，避免被当作资产上传）。
6. 分别执行 `nfpm package -p deb|rpm|archlinux -f "$RUNNER_TEMP/nfpm.yaml" -t dist/`。
7. 校验：
   - 三种扩展名各恰好一个；
   - `.deb` 内含 `/usr/bin/rime-type-stats`；
   - Arch 包内含 systemd 用户单元；
   - 安装 `rpm` 后校验 RPM 依赖包含 `gtk3`。
8. 上传 artifact `linux-x86_64-<version>`。

`publish` job 改为 `needs: [build-dmg, build-linux]`，增加下载 Linux artifact 的步骤，发布文件改为显式 glob：`dist/*.dmg dist/*.deb dist/*.rpm dist/*.pkg.tar.zst`，其余发布逻辑（预发布判断、覆盖资产）不变。

### 3.4 包内容与元数据

新增 `packaging/nfpm.yaml`：

- name：`rime-type-stats`
- arch：`amd64`（nfpm 自动映射 deb `amd64`、rpm/Arch `x86_64`）
- version：`__VERSION__`，release：`1`
- section：`utils`，priority：`optional`
- maintainer：`lovelock <lovelock@users.noreply.github.com>`
- vendor / homepage：`lovelock` / `https://github.com/lovelock/rime-type-stats`
- license：`MIT`
- 文件：
  - `target/release/rime-type-stats` → `/usr/bin/rime-type-stats`
  - `packaging/rime-type-stats.service` → `/usr/lib/systemd/user/rime-type-stats.service`
  - `LICENSE` → `/usr/share/licenses/rime-type-stats/LICENSE`
- 依赖按格式覆盖：
  - deb：`libgtk-3-0 | libgtk-3-0t64`, `libayatana-appindicator3-1`（兼容 Debian 12 与 Debian 13/Ubuntu 24.04+ 的 t64 改名）
  - rpm：`gtk3`, `libayatana-appindicator-gtk3`（Fedora 直接可用；RHEL 9 需启用 EPEL，README 注明）
  - archlinux：`gtk3`, `libayatana-appindicator`

新增 `packaging/rime-type-stats.service`：

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

包只安装单元，不自动 enable；用户执行 `systemctl --user enable --now rime-type-stats`。

### 3.5 LICENSE 与 Cargo 元数据

- 新增根目录 `LICENSE`（MIT，版权署名 `2026 rime-type-stats contributors`）。
- `Cargo.toml` 增加 `description`、`license = "MIT"`、`repository`。

### 3.6 安装方式共存

Linux 同时保留两条安装路径，但以发行版包为主：

- 包安装：`/usr/bin` + `/usr/lib/systemd/user`，推荐给普通用户。
- 源码脚本：`scripts/install.sh` 仍可用于未打包的发行版或开发环境，装到用户目录并启用用户级单元。

README 明确要求二选一：如果之前跑过 Linux 安装脚本，先 `scripts/uninstall.sh` 再安装发行版包，避免 `~/.config/systemd/user/rime-type-stats.service` 遮蔽包内的同名单元。不做自动迁移。

### 3.7 文档

README 增加「Linux 安装包」小节：

- 从 Releases 下载对应包；
- Debian/Ubuntu：`sudo apt install ./rime-type-stats_*.deb`
- Fedora/RHEL：`sudo dnf install ./rime-type-stats-*.rpm`（RHEL 9 需 EPEL）
- Arch：`sudo pacman -U ./rime-type-stats-*.pkg.tar.zst`
- 然后 `systemctl --user enable --now rime-type-stats`
- 说明 x86_64 only、运行时依赖由包管理器处理。
- 说明源码脚本与发行版包二选一，以及迁移步骤。

## 4. 明确不做

- aarch64 Linux 包
- AUR 提交、apt/yum 仓库托管
- 包签名（deb/rpm 签名、Arch 包签名）
- 自动 enable 用户服务
- 修改 macOS DMG 构建逻辑

## 5. 风险与缓解

| 风险 | 缓解 |
|---|---|
| rebase 冲突漏掉上游改动 | 逐提交解决冲突；完成后 `git diff origin/main` 审阅；macOS 专属代码只做 cfg 隔离 |
| 旧名残留导致运行时路径错乱 | rebase 后 `grep -R 'rime-counter\|RIME_COUNTER'` 清零 |
| nfpm 预发布版本在 Arch 上元数据异常 | 统一把 `-` 换成 `.` 再传给 nfpm，并在本地用 v2.48.0 实测 |
| 不同发行版依赖包名不一致 | deb 用 `libgtk-3-0 \| libgtk-3-0t64` 兼容 t64 改名；RHEL 的 EPEL 依赖写进 README；CI 校验 RPM requires |
| 本地 Arch 环境无法完整校验 deb/rpm | 用 `ar`/`tar` 检查 deb 内容；CI 安装 `rpm` 检查 RPM；Arch 包本地可解压运行 |
| 用户机器上的旧 `rime-counter` 服务残留 | 验证时先卸载旧服务，再安装新 `rime-type-stats` |
| 脚本安装与包安装产生同名用户单元遮蔽 | README 要求二选一并给出迁移步骤；包只安装、不自动 enable |

## 6. 验证

| 层级 | 方式 |
|---|---|
| 单元/回归 | `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test`（67 + 5） |
| 旧名清理 | `grep -R` 无 `rime-counter` / `RIME_COUNTER` |
| 打包 | 本地 nfpm v2.48.0 生成三种包，检查文件名、大小、内容列表 |
| Arch 包 | 解压后运行 `usr/bin/rime-type-stats --dump`，确认能读到本机 fcitx5 数据 |
| deb 元数据 | `ar p *.deb control.tar.gz \| tar xzO ./control` 检查 Package/Version/Depends |
| CI 语法 | actionlint（若可用）或 YAML 解析 |
| CI 预演 | `ci.yml` 的 `package-linux` job 在 PR/push 上成功产出三类包 |
| 端到端 | 本机卸载旧 `rime-counter` 服务，安装新 Arch 包，`systemctl --user enable --now`，DBus 菜单显示今日字数 |

## 7. 我做出的决定（builder check 后）

1. deb 依赖用 `libgtk-3-0 | libgtk-3-0t64`，同时覆盖 Debian 12 与 Debian 13/Ubuntu 24.04+。
2. RPM 目标为 Fedora / RHEL + EPEL，README 注明 RHEL 9 需启用 EPEL。
3. 发行版包与源码脚本并存，包为主；README 要求二选一并给出迁移步骤，不做自动迁移。
4. CI 增加 PR/push 的打包预演 job，避免只能靠推标签才发现 nfpm 配置错误。
5. 渲染后的 nfpm 配置放 `$RUNNER_TEMP`，发布只上传显式的 DMG/deb/rpm/Arch 四类文件。
