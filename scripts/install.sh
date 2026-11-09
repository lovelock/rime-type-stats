#!/usr/bin/env bash
set -euo pipefail

APP_NAME="rime-counter"
BUNDLE_ID="com.lovelock.rime-counter"
INSTALL_DIR="$HOME/Library/Application Support/rime-counter/bin"
BIN="$INSTALL_DIR/$APP_NAME"
PLIST="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"

echo "==> 编译 release 版本"
cargo build --release

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
cp "target/release/$APP_NAME" "$BIN"
chmod +x "$BIN"

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
  <string>$HOME/Library/Logs/$APP_NAME.log</string>
  <key>StandardErrorPath</key>
  <string>$HOME/Library/Logs/$APP_NAME.log</string>
</dict>
</plist>
PLIST_EOF

echo "==> 加载 LaunchAgent"
launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"

echo "==> 完成。菜单栏应已出现 ⌨ 图标。"
echo "    卸载：scripts/uninstall.sh"