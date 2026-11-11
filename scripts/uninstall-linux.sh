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
