#!/usr/bin/env bash
# 停止并移除 rime-type-stats 的 LaunchAgent 与二进制，保留统计数据。
set -euo pipefail

APP_NAME="rime-type-stats"
BUNDLE_ID="com.lovelock.rime-type-stats"
# 旧版本的 LaunchAgent 标签，卸载时一并清掉
LEGACY_BUNDLE_ID="com.lovelock.rime-counter"
PLIST="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"
LEGACY_PLIST="$HOME/Library/LaunchAgents/$LEGACY_BUNDLE_ID.plist"
BIN_DIR="$HOME/Library/Application Support/rime-type-stats/bin"

if [[ -f "$PLIST" ]]; then
  echo "==> 停止并移除 LaunchAgent"
  launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
  rm -f "$PLIST"
fi

if [[ -f "$LEGACY_PLIST" ]]; then
  echo "==> 停止并移除旧版 LaunchAgent（$LEGACY_BUNDLE_ID）"
  launchctl bootout "gui/$(id -u)/$LEGACY_BUNDLE_ID" 2>/dev/null || true
  rm -f "$LEGACY_PLIST"
fi

echo "==> 删除二进制：$BIN_DIR"
rm -rf "$BIN_DIR"

echo "==> 数据（events.jsonl）已保留：$HOME/Library/Application Support/rime-type-stats"