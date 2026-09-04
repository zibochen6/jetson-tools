#!/usr/bin/env bash
# bootstrap.sh — 幂等地在 Ubuntu(22.04/24.04) Jetson 上配置 XRDP + xorgxrdp + XFCE 虚拟桌桌面。
#
# 目标拓扑（PRD）：XRDP → xorgxrdp → Xorg(:10 虚拟) → XFCE。不依赖 HDMI / DISPLAY :0 / Wayland / GNOME。
#
# 设计约束：
#   * 幂等：重复运行不破坏系统（包已装 → 跳过；配置已对 → 跳过；服务已是 active → 跳过）
#   * 稳定优先：只装 apt distro 包，绝不 source build
#   * 可观测：向 stdout 输出 `[bootstrap]` 前缀的阶段行，Phase 3 后端据此渲染进度
#   * 凭据安全：需要 sudo 时用 `sudo -S -p ''` 从 stdin 读密码（由上层 SSH channel 喂入），
#     密码绝不出现在 argv / 日志。若已是 root 则直接执行。
#
# 用法：
#   本机免密 sudo：        ./bootstrap.sh
#   远程（sudo 密码经 ssh stdin 喂入，脚本经文件执行）：
#       scp bootstrap.sh user@jetson:/tmp/jr-bootstrap.sh
#       printf '%s\n' "$SUDO_PASS" | ssh user@jetson 'bash /tmp/jr-bootstrap.sh'

set -uo pipefail

log() { printf '[bootstrap] %s\n' "$1"; }

# ---------- root 执行器 ----------
# 若已是 root 则直接执行；否则优先 sudo -n（免密）；再退化为 sudo -S 从 stdin 读密码一次。
# 密码仅存于 shell 变量，经管道送入 sudo -S，绝不出现在 argv / 日志。
# 注意：需要密码时脚本会从 stdin 读一行（脚本本身需以文件方式执行，stdin 留给密码）。
if [ "$(id -u)" -eq 0 ]; then
  as_root() { "$@"; }
elif sudo -n true 2>/dev/null; then
  as_root() { sudo "$@"; }
else
  IFS= read -r SUDO_PASSWORD || true
  as_root() { printf '%s\n' "$SUDO_PASSWORD" | sudo -S -p '' "$@"; }
fi

# The script is launched through sudo after preflight, but .xsession belongs
# to the SSH login user, not root. SUDO_USER is set by sudo; resolve its home
# from passwd rather than trusting root's HOME.
SESSION_USER="${SUDO_USER:-$(id -un)}"
SESSION_HOME="$(getent passwd "$SESSION_USER" 2>/dev/null | cut -d: -f6)"
if [ -z "$SESSION_HOME" ]; then
  SESSION_HOME="$HOME"
fi
SESSION_GROUP="$(id -gn "$SESSION_USER" 2>/dev/null || id -gn)"
session_chown() {
  if [ "$(id -u)" -eq 0 ]; then
    chown "$SESSION_USER:$SESSION_GROUP" "$@"
  fi
}

log "phase=start user=$SESSION_USER root=$([ "$(id -u)" -eq 0 ] && echo yes || echo no)"

# ---------- 1. 发行版识别（决定包名分支） ----------
UBUNTU_VERSION_ID="$(grep -E '^VERSION_ID=' /etc/os-release | cut -d= -f2 | tr -d '"')"
case "$UBUNTU_VERSION_ID" in
  22.04|24.04) log "step=detect_os ubuntu=$UBUNTU_VERSION_ID" ;;
  *) log "step=error unsupported_os=$UBUNTU_VERSION_ID"; exit 2 ;;
esac

# ---------- 2. 安装软件包（幂等：逐包检查） ----------
log "step=install_packages status=start"
PKGS="xrdp xorgxrdp xfce4 xfce4-terminal dbus-x11 ssl-cert"
MISSING=""
for p in $PKGS; do
  if dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed'; then
    log "  package=$p already_installed"
  else
    MISSING="$MISSING $p"
  fi
done
if [ -n "$MISSING" ]; then
  log "  installing:$MISSING"
  as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y $MISSING \
    || { log "step=error apt_install_failed"; exit 3; }
else
  log "  all packages already installed"
fi
log "step=install_packages status=done"

# ---------- 3. 配置登录用户会话：~/.xsession（独立 D-Bus 会话总线）----------
# 必须 dbus-launch 起每会话私有总线：否则同一用户的所有会话共享
# /run/user/$UID/bus（systemd user bus），D-Bus 单实例应用（gnome-terminal 等）
# 会把窗口开往 server 实例所在会话（物理显示器侧）。
log "step=configure_session status=start"
if [ ! -f "$SESSION_HOME/.xsession" ] || ! grep -q 'use_compositing' "$SESSION_HOME/.xsession" 2>/dev/null; then
  cat > "$SESSION_HOME/.xsession" << 'XEOF'
eval $(dbus-launch --exit-with-session)
export DBUS_SESSION_BUS_ADDRESS
# xfwm4 在 xorgxrdp 虚拟显示上开 GL 合成会冻死（窗口拖不动/最大化无响应），会话启动即关
xfconf-query -c xfwm4 -p /general/use_compositing -t bool -s false 2>/dev/null || true
exec startxfce4
XEOF
  session_chown "$SESSION_HOME/.xsession"
  log "  wrote $SESSION_HOME/.xsession (private bus + compositor off)"
else
  log "  $SESSION_HOME/.xsession already correct"
fi
chmod u+x "$SESSION_HOME/.xsession" 2>/dev/null || true

# dbus/xdg 运行时目录（无显示器环境下 session 启动需要，幂等创建）
mkdir -p "$SESSION_HOME/.config" 2>/dev/null || true
session_chown "$SESSION_HOME/.config"
log "step=configure_session status=done"

# ---------- 3b. 自愈：禁用语法错误的 ~/.xsessionrc ----------
# NVIDIA JetPack 镜像自带的 ~/.xsessionrc 含 bash 数组语法，而 /etc/X11/Xsession 用
# sh(dash) source 它，会导致 Xsession 链中断（XFCE 会话 0 秒退出）。检测并重命名禁用（保留原件，可逆）。
log "step=fix_xsessionrc status=start"
if [ -f "$SESSION_HOME/.xsessionrc" ] && ! sh -n "$SESSION_HOME/.xsessionrc" 2>/dev/null; then
  mv "$SESSION_HOME/.xsessionrc" "$SESSION_HOME/.xsessionrc.disabled-by-jetson-remote"
  session_chown "$SESSION_HOME/.xsessionrc.disabled-by-jetson-remote"
  log "  $SESSION_HOME/.xsessionrc has sh syntax error (bash-only); renamed aside"
elif [ -f "$SESSION_HOME/.xsessionrc" ]; then
  log "  $SESSION_HOME/.xsessionrc syntax ok, leave as-is"
else
  log "  no $SESSION_HOME/.xsessionrc"
fi
log "step=fix_xsessionrc status=done"

# ---------- 4. 禁用 Wayland（仅当存在 gdm3；headless 无 gdm 时跳过） ----------
if [ -f /etc/gdm3/custom.conf ]; then
  log "step=disable_wayland status=start"
  if grep -q '^WaylandEnable=false' /etc/gdm3/custom.conf; then
    log "  WaylandEnable already false"
  else
    as_root sed -i 's/^#\?WaylandEnable=.*/WaylandEnable=false/' /etc/gdm3/custom.conf
    log "  set WaylandEnable=false"
  fi
  log "step=disable_wayland status=done"
else
  log "step=disable_wayland status=skip (no gdm3)"
fi

# ---------- 5. 修复 XRDP TLS 私钥访问 + 启动服务（幂等） ----------
log "step=enable_service status=start"
as_root systemctl enable xrdp xrdp-sesman >/dev/null 2>&1 || true
XRDP_GROUP_CHANGED="false"
if ! id -nG xrdp 2>/dev/null | tr ' ' '\n' | grep -qx 'ssl-cert'; then
  as_root adduser xrdp ssl-cert >/dev/null 2>&1 \
    || { log "step=error xrdp_ssl_cert_group_failed"; exit 5; }
  XRDP_GROUP_CHANGED="true"
  log "  added xrdp to ssl-cert (TLS key access)"
fi
if [ "$XRDP_GROUP_CHANGED" = "true" ]; then
  as_root systemctl restart xrdp-sesman xrdp
  log "  xrdp-sesman + xrdp restarted after group update"
elif systemctl is-active --quiet xrdp && systemctl is-active --quiet xrdp-sesman; then
  log "  xrdp-sesman + xrdp already active"
else
  as_root systemctl restart xrdp-sesman xrdp
  log "  xrdp-sesman + xrdp started"
fi
log "step=enable_service status=done"

# ---------- 6. 验证 ----------
log "step=verify status=start"
sleep 1
if systemctl is-active --quiet xrdp; then
  ACTIVE=yes
else
  ACTIVE=no
fi
if systemctl is-active --quiet xrdp-sesman; then
  SESMAN_ACTIVE=yes
else
  SESMAN_ACTIVE=no
fi
if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':3389 '; then
  PORT=3389_listening
else
  PORT=3389_not_listening
fi
if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':3350 '; then
  SESMAN_PORT=3350_listening
else
  SESMAN_PORT=3350_not_listening
fi
if id -nG xrdp 2>/dev/null | tr ' ' '\n' | grep -qx 'ssl-cert'; then
  TLS_GROUP=yes
else
  TLS_GROUP=no
fi
XRDP_VER="$(dpkg-query -W -f='${Version}' xrdp 2>/dev/null || echo unknown)"
XORGXRDP_VER="$(dpkg-query -W -f='${Version}' xorgxrdp 2>/dev/null || echo unknown)"
log "  xrdp=$XRDP_VER xorgxrdp=$XORGXRDP_VER active=$ACTIVE port=$PORT sesman_active=$SESMAN_ACTIVE sesman_port=$SESMAN_PORT ssl_cert_group=$TLS_GROUP"
if [ "$ACTIVE" = yes ] && [ "$PORT" = "3389_listening" ] \
   && [ "$SESMAN_ACTIVE" = yes ] && [ "$SESMAN_PORT" = "3350_listening" ] \
   && [ "$TLS_GROUP" = yes ]; then
  log "step=verify status=done ready=true"
  log "phase=done ready=true"
  exit 0
else
  log "step=verify status=failed ready=false"
  log "phase=done ready=false"
  exit 4
fi
