#!/usr/bin/env bash
# 编译、安装并注册 rime-type-stats 为登录自启的菜单栏应用。
set -euo pipefail

APP_NAME="rime-type-stats"
BUNDLE_ID="com.lovelock.rime-type-stats"
# 旧版本的 LaunchAgent 标签，安装时一并清掉，否则同一时刻会跑两个实例
LEGACY_BUNDLE_ID="com.lovelock.rime-counter"
INSTALL_DIR="$HOME/Library/Application Support/$APP_NAME/bin"
BIN="$INSTALL_DIR/$APP_NAME"
PLIST="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"
LOG="$HOME/Library/Logs/$APP_NAME.log"

cd "$(dirname "$0")/.."

echo "==> 清理旧版 LaunchAgent（$LEGACY_BUNDLE_ID）"
launchctl bootout "gui/$(id -u)/$LEGACY_BUNDLE_ID" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/$LEGACY_BUNDLE_ID.plist"

echo "==> 编译 release 版本"
cargo build --release

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
# 必须先删除再拷贝：原地覆盖可执行文件会破坏 macOS 的代码签名缓存，
# LaunchAgent 随后会以 OS_REASON_CODESIGNING 失败退出。
rm -f "$BIN"
cp "target/release/$APP_NAME" "$BIN"
chmod +x "$BIN"

echo "==> 重新做 ad-hoc 签名"
codesign --force --sign - "$BIN"

echo "==> 写入 LaunchAgent：$PLIST"
mkdir -p "$HOME/Library/LaunchAgents"
cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$BUNDLE_ID</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>$LOG</string>
  <key>StandardErrorPath</key>
  <string>$LOG</string>
</dict>
</plist>
PLIST_EOF

echo "==> 加载 LaunchAgent"
launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
# bootout 是异步的，作业未完全卸载就 bootstrap 会返回 "Input/output error"
for _ in $(seq 1 40); do
  if ! launchctl print "gui/$(id -u)/$BUNDLE_ID" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
launchctl bootstrap "gui/$(id -u)" "$PLIST"

echo "==> 等待服务就绪"
for _ in $(seq 1 20); do
  if launchctl print "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null | grep -q 'state = running'; then
    PORT=$(grep -o 'http://127.0.0.1:[0-9]*' "$LOG" 2>/dev/null | tail -1)
    echo "==> 已启动。${PORT:-启动中，稍后查看 $LOG}"
    echo "    菜单栏应已出现 ⌨ 图标；卸载：scripts/uninstall.sh"
    exit 0
  fi
  sleep 0.5
done

echo "!! 服务未能进入 running 状态，请查看日志：$LOG" >&2
exit 1