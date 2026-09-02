#!/usr/bin/env bash
# check-environment.sh — 只读探测远程桌面环境状况，向 stdout 输出单一 JSON。
# 无 sudo、不改系统；只返回「事实」，state 由 Rust classify() 计算。
# 与 detect.sh 同模式：stdout = 唯一 JSON（供 Rust serde_json 解析），stderr 仅供诊断。
#
# 用法：ssh user@jetson 'bash -s' < check-environment.sh

set -u

pkg_installed() {
  dpkg-query -W -f='${Status}' "$1" >/dev/null 2>&1 \
    && dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q 'install ok installed'
}
pkg_version() { dpkg-query -W -f='${Version}' "$1" 2>/dev/null; }

XRDP_INSTALLED="false"
XRDP_VERSION=""
if pkg_installed xrdp; then
  XRDP_INSTALLED="true"
  XRDP_VERSION="$(pkg_version xrdp)"
fi

XORGXRDP_INSTALLED="false"
XORGXRDP_VERSION=""
if pkg_installed xorgxrdp; then
  XORGXRDP_INSTALLED="true"
  XORGXRDP_VERSION="$(pkg_version xorgxrdp)"
fi

XFCE_INSTALLED="false"
if pkg_installed xfce4-session || pkg_installed xfce4; then
  XFCE_INSTALLED="true"
fi

XRDP_ENABLED="false"
if systemctl is-enabled xrdp >/dev/null 2>&1; then
  XRDP_ENABLED="true"
fi

XRDP_ACTIVE="false"
if systemctl is-active --quiet xrdp 2>/dev/null; then
  XRDP_ACTIVE="true"
fi

PORT_3389_LISTENING="false"
if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':3389 '; then
  PORT_3389_LISTENING="true"
fi

# 会话配置：~/.xsession 含 startxfce4
SESSION_CONFIGURED="false"
if [ -f "$HOME/.xsession" ] && grep -q 'startxfce4' "$HOME/.xsession" 2>/dev/null; then
  SESSION_CONFIGURED="true"
fi

# 兼容性：~/.xsessionrc 无 sh 语法错误（文件不存在视为 OK）
XSESSIONRC_OK="true"
if [ -f "$HOME/.xsessionrc" ] && ! sh -n "$HOME/.xsessionrc" 2>/dev/null; then
  XSESSIONRC_OK="false"
fi

export XRDP_INSTALLED XRDP_VERSION XORGXRDP_INSTALLED XORGXRDP_VERSION \
       XFCE_INSTALLED XRDP_ENABLED XRDP_ACTIVE PORT_3389_LISTENING \
       SESSION_CONFIGURED XSESSIONRC_OK

python3 - <<'PY'
import json
import os

def b(k):
    return os.environ.get(k, "false") == "true"

def s(k):
    return os.environ.get(k, "") or ""

print(json.dumps({
    "xrdp_installed": b("XRDP_INSTALLED"),
    "xrdp_version": s("XRDP_VERSION"),
    "xorgxrdp_installed": b("XORGXRDP_INSTALLED"),
    "xorgxrdp_version": s("XORGXRDP_VERSION"),
    "xfce_installed": b("XFCE_INSTALLED"),
    "xrdp_enabled": b("XRDP_ENABLED"),
    "xrdp_active": b("XRDP_ACTIVE"),
    "port_3389_listening": b("PORT_3389_LISTENING"),
    "session_configured": b("SESSION_CONFIGURED"),
    "xsessionrc_ok": b("XSESSIONRC_OK"),
}, ensure_ascii=False, indent=2))
PY