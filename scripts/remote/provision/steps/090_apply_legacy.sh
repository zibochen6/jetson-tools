#!/usr/bin/env bash
# step: provision.core — 执行 legacy bootstrap 的完整自愈安装体（不重写能力）。
# 本阶段 check/verify 用轻量事实判定（与 legacy 验证一致），apply 委托
# legacy bootstrap 完整流程；等真机阶段再细拆各子步骤。

step_id="provision.core"
step_register "$step_id" 1
[ -n "${BOOTSTRAP_PATH:-}" ] || BOOTSTRAP_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/bootstrap.sh"

step_provision_core_check() {
  if [ ! -f "$BOOTSTRAP_PATH" ]; then
    echo "unknown"
    return 1
  fi
  if [ -f "$BOOTSTRAP_PATH" ] && bash -n "$BOOTSTRAP_PATH" 2>/dev/null; then
    echo "needs_change"
    return 0
  fi
  echo "unknown"
  return 1
}

step_provision_core_apply() {
  bash "$BOOTSTRAP_PATH" >/dev/null 2>&1
}

step_provision_core_verify() {
  # 真机阶段初版：检查服务均 active 且 3389/3350 监听（legacy verify 的子集）。
  if systemctl is-active --quiet xrdp 2>/dev/null \
     && systemctl is-active --quiet xrdp-sesman 2>/dev/null \
     && ss -ltn 2>/dev/null | grep -q ':3389 ' \
     && ss -ltn 2>/dev/null | grep -q ':3350 '; then
    echo "satisfied"
    return 0
  fi
  echo "needs_change"
  return 1
}