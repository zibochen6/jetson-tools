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

XRDP_SESMAN_ACTIVE="false"
if systemctl is-active --quiet xrdp-sesman 2>/dev/null; then
  XRDP_SESMAN_ACTIVE="true"
fi

PORT_3389_LISTENING="false"
if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':3389 '; then
  PORT_3389_LISTENING="true"
fi

PORT_3350_LISTENING="false"
if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':3350 '; then
  PORT_3350_LISTENING="true"
fi

# Ubuntu's default /etc/xrdp/key.pem ultimately lives below
# /etc/ssl/private (root:ssl-cert, 0710). xrdp can listen on 3389 without
# this membership, but every incoming TLS handshake then fails. Checking the
# account's declared groups is read-only and works without sudo.
XRDP_IN_SSL_CERT_GROUP="false"
if id -nG xrdp 2>/dev/null | tr ' ' '\n' | grep -qx 'ssl-cert'; then
  XRDP_IN_SSL_CERT_GROUP="true"
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

# KI-038：xrdp 0.9.17 的 connect_loopback 对 "127.0.0.1" 目标先尝试 ::1（非阻塞
# connect 返回 EINPROGRESS 即返回），lo 丢失 ::1 后该连接永不完成、v4 回退分支
# 永远不会执行——xrdp 永远连不上 sesman（端口监听但零包）。常见诱因：
# /etc/sysctl.conf 手动禁用 IPv6（all/default.disable_ipv6=1）会把 lo 的 ::1 抹掉。
LO_IPV6_LOOPBACK="false"
if ip -6 addr show lo 2>/dev/null | grep -q 'inet6 ::1/128'; then
  LO_IPV6_LOOPBACK="true"
fi

# KI-044：远程会话（xrdp）下的 NetworkManager 授权。Ubuntu 自带的 NM polkit 规则
# 只对 subject.local && subject.active 放行，而 xrdp 是 remote 会话（local=false），
# 规则不匹配 → 落到 NM policy 默认值：enable-disable-network 的 allow_inactive=no
# 直接拒绝（远程桌面里改 WiFi 报 "Not authorized to control networking"），
# settings.modify.system 则要求交互认证而会话内没有 polkit agent。
# pkcheck 取当前进程的判定：修复（50-local.d 的 .pkla）落盘后此处应为 true。
NM_POLKIT_OK="false"
if command -v pkcheck >/dev/null 2>&1 \
   && pkcheck --action-id org.freedesktop.NetworkManager.enable-disable-network --process $$ >/dev/null 2>&1; then
  NM_POLKIT_OK="true"
fi

# KI-045：sesman 必须监听 127.0.0.1，否则远程桌面的剪贴板必死。
# xrdp 0.9.17 chansrv_config.c 的 use_unix_socket **只**由这一条推导：
#     if (name == "ListenAddress" && value == "127.0.0.1") use_unix_socket = 1;
# （默认 0，键缺失同样为 0。）为 0 时 chansrv 转去监听 TCP `7200 + display`，
# 而 xrdp 前端只连 /run/xrdp/sockdir/xrdp_chansrv_socket_<display> → 握手永远失败
# → CLIPRDR 永不协商 → 剪贴板双向全死，且重连救不回来。
# 这条是**静态可判**的（不像 chansrv 进程/socket 只在活跃会话期间存在），
# 所以用它当分类依据，而不是去猜运行时的 chansrv 状态。
# 注意 chansrv 接受连接后会主动删掉自己的监听 socket，运行期查那个文件并不可靠。
SESMAN_LISTEN_LOCAL="false"
if grep -qE '^[[:space:]]*ListenAddress[[:space:]]*=[[:space:]]*127\.0\.0\.1[[:space:]]*$' \
     /etc/xrdp/sesman.ini 2>/dev/null; then
  SESMAN_LISTEN_LOCAL="true"
fi

export XRDP_INSTALLED XRDP_VERSION XORGXRDP_INSTALLED XORGXRDP_VERSION \
       XFCE_INSTALLED XRDP_ENABLED XRDP_ACTIVE XRDP_SESMAN_ACTIVE \
       PORT_3389_LISTENING PORT_3350_LISTENING \
       XRDP_IN_SSL_CERT_GROUP SESSION_CONFIGURED XSESSIONRC_OK LO_IPV6_LOOPBACK \
       NM_POLKIT_OK SESMAN_LISTEN_LOCAL

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
    "xrdp_sesman_active": b("XRDP_SESMAN_ACTIVE"),
    "port_3389_listening": b("PORT_3389_LISTENING"),
    "port_3350_listening": b("PORT_3350_LISTENING"),
    "xrdp_in_ssl_cert_group": b("XRDP_IN_SSL_CERT_GROUP"),
    "session_configured": b("SESSION_CONFIGURED"),
    "xsessionrc_ok": b("XSESSIONRC_OK"),
    "lo_ipv6_loopback": b("LO_IPV6_LOOPBACK"),
    "nm_polkit_ok": b("NM_POLKIT_OK"),
    "sesman_listen_local": b("SESMAN_LISTEN_LOCAL"),
}, ensure_ascii=False, indent=2))
PY
