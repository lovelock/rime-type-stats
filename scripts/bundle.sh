#!/usr/bin/env bash
# 把 release 二进制打包成 rime-type-stats.app 并生成 DMG。
#
#   scripts/bundle.sh [版本号]
#
# 版本号缺省取 `git describe`，用于 CFBundleShortVersionString。
# 产物：dist/rime-type-stats_<版本>_<架构>.dmg
#
# 打包名跟仓库名走（rime-type-stats）；crate、二进制、bundle id、LaunchAgent 标签
# 与数据目录也统一用 rime-type-stats。
#
# 只做 ad-hoc 签名：没有 Developer ID，因此用户首次打开需要在
# 「系统设置 → 隐私与安全性」放行一次，或右键 →「打开」。
set -euo pipefail

APP_NAME="rime-type-stats"
DISPLAY_NAME="Rime Type Stats"
EXECUTABLE="rime-type-stats"
BUNDLE_ID="com.lovelock.rime-type-stats"
MIN_MACOS="11.0"

cd "$(dirname "$0")/.."

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  VERSION="$(git describe --tags --always --dirty 2>/dev/null || echo 0.0.0)"
  VERSION="${VERSION#v}"
fi
VERSION="${VERSION#v}"

ARCH="$(uname -m)"
if [ "$ARCH" != "arm64" ]; then
  echo "!! 需要 Apple Silicon 构建机，当前架构：$ARCH" >&2
  exit 1
fi

BIN="target/release/$EXECUTABLE"
# 无条件调用：cargo 自身是增量的，这样不会因为残留的旧 target 目录而把
# 上一个提交的二进制打进新版本。
echo "==> 编译 release 版本"
cargo build --release --locked

DIST="dist"
APP="$DIST/$APP_NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/$EXECUTABLE"
chmod +x "$APP/Contents/MacOS/$EXECUTABLE"

if [ -f assets/AppIcon.icns ]; then
  cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi

cat > "$APP/Contents/Info.plist" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>$APP_NAME</string>
  <key>CFBundleDisplayName</key>
  <string>$DISPLAY_NAME</string>
  <key>CFBundleIdentifier</key>
  <string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key>
  <string>$EXECUTABLE</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>$MIN_MACOS</string>
  <!-- 菜单栏常驻：不在 Dock / 程序切换器里出现 -->
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST_EOF

echo "==> ad-hoc 签名"
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --deep --strict "$APP"

echo "==> 校验架构与版本"
lipo -archs "$APP/Contents/MacOS/$EXECUTABLE" | grep -qx arm64
/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist"

DMG="$DIST/${APP_NAME}_${VERSION}_${ARCH}.dmg"
STAGE="$DIST/stage"
rm -rf "$STAGE" "$DMG"
# 清掉同一应用的历史产物，避免本地多次打包后 dist/ 里堆出多个 DMG，
# 让下游「恰好一个 DMG」的校验失效。
rm -f "$DIST/${APP_NAME}"_*.dmg
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

echo "==> 生成 $DMG"
hdiutil create \
  -volname "$DISPLAY_NAME" \
  -srcfolder "$STAGE" \
  -fs HFS+ \
  -format UDZO \
  -ov \
  "$DMG" >/dev/null

rm -rf "$STAGE"

echo "==> 完成：$DMG"
ls -lh "$DMG"