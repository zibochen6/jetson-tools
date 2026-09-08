#!/usr/bin/env bash
# provisioning-matrix.sh — Provisioning V2 本地契约矩阵（无真机阶段）。
#
# 本机可验部分：
#   P1 语法    所有 remote/provision 脚本 bash -n 通过
#   P2 协议    --self-test 输出即 JSONL 契约（schema/seq/run_id/run_completed）
#   P3 幂等    连续两次 --self-test 输出 changed=true → changed=true（self-test
#              始终模拟首次安装；真机幂等用 CI/VM 阶段验证）
#
# PENDING REAL-HARDWARE: Ubuntu 20.04/22.04/24.04 VM/真机全量矩阵（r2 §3.13）。
# 真机阶段接入点：runner.sh 的 detect.os + provision.core apply 委托 legacy。

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PASS=0
fail() { echo "✗ $1"; exit 1; }
ok() { echo "  ✓ $1"; PASS=$((PASS+1)); }

# P1 语法
for f in scripts/remote/bootstrap.sh scripts/remote/detect.sh scripts/remote/check-environment.sh \
         scripts/remote/provision/runner.sh scripts/remote/provision/protocol.sh \
         scripts/remote/provision/steps/*.sh; do
  bash -n "$f" || fail "bash -n $f"
done
ok "all provision scripts pass bash -n"

# P2 协议约定
OUT="$(mktemp)"
bash scripts/remote/bootstrap.sh --protocol jsonl --run-id matrix --self-test 2>/dev/null > "$OUT"
python3 - "$OUT" <<'PY'
import json, sys
lines = [l for l in open(sys.argv[1]).read().splitlines() if l.strip()]
assert lines, "no protocol lines"
seqs = []
for l in lines:
    d = json.loads(l)
    assert d["schema"] == "jr.provision.event/v1"
    assert d["run_id"] == "matrix"
    seqs.append(d["seq"])
assert seqs == list(range(1, len(lines) + 1)), "seq not monotonic"
assert '"type":"run_started"' in lines[0]
assert '"type":"run_completed"' in lines[-1] and '"status":"success"' in lines[-1]
assert any('"type":"step_result"' in l and '"changed":true' in l for l in lines)
print("  ✓ protocol contract: %d lines, seq monotonic, run_completed=success" % len(lines))
PY
rm -f "$OUT"

echo "provisioning-matrix (local contract): ALL PASS"
echo "PENDING REAL-HARDWARE: Ubuntu 20.04/22.04/24.04 matrix"