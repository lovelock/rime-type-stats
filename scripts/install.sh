#!/usr/bin/env bash
# 按平台分派安装脚本。
set -euo pipefail

case "$(uname -s)" in
  Darwin) exec "$(dirname "$0")/install-macos.sh" ;;
  Linux) exec "$(dirname "$0")/install-linux.sh" ;;
  *)
    echo "不支持的系统：$(uname -s)" >&2
    exit 1
    ;;
esac
