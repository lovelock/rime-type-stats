#!/usr/bin/env bash
# 编译、安装并注册 rime-counter 为 systemd 用户服务。
set -euo pipefail

APP_NAME="rime-counter"
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
