#!/usr/bin/env bash
# 停止并移除 rime-counter 的 LaunchAgent 与二进制，保留统计数据。
set -euo pipefail

APP_NAME="rime-counter"
BUNDLE_ID="com.lovelock.rime-counter"
PLIST="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"
BIN_DIR="$HOME/Library/Application Support/rime-counter/bin"

if [[ -f "$PLIST" ]]; then
  echo "==> 停止并移除 LaunchAgent"
  launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
  rm -f "$PLIST"
fi

echo "==> 删除二进制：$BIN_DIR"
rm -rf "$BIN_DIR"

echo "==> 数据（events.jsonl）已保留：$HOME/Library/Application Support/rime-counter"