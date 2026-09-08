#!/usr/bin/env bash
# provision/protocol.sh — jr.provision.event/v1 JSONL 输出工具。
# 契约：stdout 只出协议；本脚本自身日志走 stderr。
# 每次调用 emit() 自动生成 +1 的 seq；run 上下文由 runner 注入。

# 由 runner.sh source；典型未设时给安全默认
[ -n "${JR_SCHEMA:-}" ]     || JR_SCHEMA="jr.provision.event/v1"
[ -n "${JR_RUN_ID:-}" ]     || JR_RUN_ID="unknown-run"
[ -n "${JR_PROVISIONER:-}" ] || JR_PROVISIONER="0.3.8"

# 全局序号（runner 通过子 shell 不可变；直接 source 调用共享变量）
JR_EMIT_SEQ="$(printf '%s' "${JR_EMIT_SEQ:-0}")"

emit() {
  # $1 = JSON 对象（不含右花括号？——不行，保持简单：$1 是完整 JSON 对象）
  JR_EMIT_SEQ=$((JR_EMIT_SEQ + 1))
  ts="$(date +%s)"
  # shell 拼接：seq 由本函数控制，调用方不得再给 seq
  printf '{"schema":"%s","seq":%s,"run_id":"%s","ts":%s,%s}\n' \
    "$JR_SCHEMA" "$JR_EMIT_SEQ" "$JR_RUN_ID" "$ts" "$1"
}

emit_std() {
  # $1 无外层引号字段串（含 type）
  emit "\"type\":\"$1\",$2"
}

log_err() { printf '[runner] %s\n' "$1" >&2; }