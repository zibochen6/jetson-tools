#!/usr/bin/env bash
# provision/runner.sh — Provisioning V2 step 引擎（jr.provision.event/v1 JSONL）。
#
# 用法：bash runner.sh --run-id <id> [--self-test]
# stdout 只出 JSONL 协议；[runner] 日志走 stderr。
# step 模型：每个 step 注册 (id, version, check_fn, apply_fn, verify_fn)；
# check→needs_change 时依次 apply、verify；结果一律 emit step_result。
# 幂等由结果证明（changed=false + apply=not_run），不声明 idempotent:true。

set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=protocol.sh
. "$SCRIPT_DIR/protocol.sh"

JR_RUN_ID=""
SELF_TEST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --run-id) JR_RUN_ID="$2"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    *) shift ;;
  esac
done
[ -n "$JR_RUN_ID" ] || JR_RUN_ID="run-$$"
export JR_RUN_ID

json_string() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

step_registry=""

step_register() {
  local id="$1" version="$2"
  step_registry="$step_registry $id:$version"
}
step_exists() { case " $step_registry " in *" $1:"*) return 0 ;; *) return 1 ;; esac }

# step source 文件约定：定义 step_<id>_check / step_<id>_apply / step_<id>_verify
# （id 中的 '.' 转 '_'），并调 step_register <id> <version>。
for f in "$SCRIPT_DIR"/steps/*.sh; do
  [ -f "$f" ] || continue
  # shellcheck disable=SC1090
  . "$f"
done

emit "\"type\":\"run_started\",\"provisioner_version\":\"$(json_string "$JR_PROVISIONER")\",\"steps\":\"$(json_string "${step_registry# }")\""

run_changed=0
for entry in $step_registry; do
  step_id="${entry%%:*}"
  fname="$(printf '%s' "$step_id" | tr '.' '_')"
  emit "\"type\":\"step_started\",\"step_id\":\"$(json_string "$step_id")\",\"step_version\":${entry##*:}"

  check_fn="step_${fname}_check"
  apply_fn="step_${fname}_apply"
  verify_fn="step_${fname}_verify"
  [ "$(type -t "$check_fn" 2>/dev/null)" = "function" ] || check_fn=""
  [ "$(type -t "$apply_fn" 2>/dev/null)" = "function" ] || apply_fn=""
  [ "$(type -t "$verify_fn" 2>/dev/null)" = "function" ] || verify_fn=""

  # check
  if [ -n "$SELF_TEST" ]; then
    check_out="needs_change"
  elif [ -n "$check_fn" ]; then
    if out="$("$check_fn" 2>/dev/null)"; then
      check_out="${out:-satisfied}"
    else
      check_out="failed"
    fi
  else
    check_out="unknown"
  fi

  # 判定 apply
  if [ "$check_out" = "needs_change" ] || [ "$check_out" = "failed" ]; then
    if [ -n "$SELF_TEST" ]; then
      apply_out="changed" && run_changed=1
    elif [ -n "$apply_fn" ]; then
      if "$apply_fn" >/dev/null 2>&1; then
        apply_out="changed" && run_changed=1
      else
        apply_out="failed"
      fi
    else
      apply_out="noop"
    fi
  else
    apply_out="not_run"
  fi

  # verify
  if [ "$apply_out" = "failed" ]; then
    verify_out="not_run"
  elif [ -n "$SELF_TEST" ]; then
    verify_out="passed"
  elif [ -n "$verify_fn" ]; then
    if "$verify_fn" >/dev/null 2>&1; then
      verify_out="passed"
    else
      verify_out="failed"
    fi
  else
    verify_out="not_run"
  fi

  emit "\"type\":\"step_result\",\"step_id\":\"$(json_string "$step_id")\",\"step_version\":${entry##*:},\"check\":\"$check_out\",\"apply\":\"$apply_out\",\"verify\":\"$verify_out\",\"changed\":$([ "$apply_out" = "changed" ] && printf true || printf false)"

  # 错误对象（step 失败）
  if [ "$apply_out" = "failed" ] || [ "$verify_out" = "failed" ] || [ "$check_out" = "failed" ]; then
    emit "\"type\":\"run_completed\",\"status\":\"failed\",\"step_id\":\"$(json_string "$step_id")\",\"error\":{\"code\":\"PROVISION_STEP_FAILED\",\"message\":\"step $(json_string "$step_id") $(json_string "$check_out/$apply_out/$verify_out")\",\"retryable\":true}"
    exit 4
  fi
done

emit "\"type\":\"run_completed\",\"status\":\"success\",\"changed\":$([ "$run_changed" = "1" ] && printf true || printf false)"
exit 0