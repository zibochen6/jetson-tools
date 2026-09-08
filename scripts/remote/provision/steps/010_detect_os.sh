#!/usr/bin/env bash
# step: detect.os — 发行版识别（Ubuntu 20.04/22.04/24.04 白名单）。
# 逻辑 ID 不含编号；输出契约：check/verify 打印 satisfied|needs_change|unknown，
# 日志（如有）一律 stderr。

step_id="detect.os"
step_register "$step_id" 1

_step_supported() {
  local v
  v="$(grep -E '^VERSION_ID=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '"')"
  case "$v" in
    20.04|22.04|24.04) return 0 ;;
    *) return 1 ;;
  esac
}

step_detect_os_check() {
  if [ ! -f /etc/os-release ]; then
    echo "unknown"
    return 1
  fi
  if _step_supported; then
    echo "satisfied"
    return 0
  fi
  echo "unknown"
  return 1
}

step_detect_os_apply() {
  # 不支持的系统无法「apply」——上层会拿到失败对象，前端据此拒绝继续安装。
  echo "not_run"
  return 0
}

step_detect_os_verify() {
  _step_supported && { echo "satisfied"; return 0; } || { echo "unknown"; return 1; }
}