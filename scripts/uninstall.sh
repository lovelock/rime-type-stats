#!/usr/bin/env bash
# 按平台分派卸载脚本。
set -euo pipefail

case "$(uname -s)" in
  Darwin) exec "$(dirname "$0")/uninstall-macos.sh" ;;
  Linux) exec "$(dirname "$0")/uninstall-linux.sh" ;;
  *)
    echo "不支持的系统：$(uname -s)" >&2
    exit 1
    ;;
esac
