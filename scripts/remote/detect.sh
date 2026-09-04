#!/usr/bin/env bash
# detect.sh — 检测 NVIDIA Jetson 设备信息，向 stdout 输出结构化 JSON。
#
# 设计约束：
#   * 只读，绝不写系统 / 不装任何东西（只探测，与 bootstrap.sh 分离）
#   * 任何主机（含非 Jetson 的 Linux）都返回 0，用 is_jetson 字段区分
#   * 不依赖 jq / 网络；JSON 由 python3 生成保证转义正确（Ubuntu 自带 python3）
#   * 单一 JSON 是 stdout 唯一内容（供 Rust 后端直接反序列化）
#
# 用法：
#   ./detect.sh            # 本机执行
#   ssh user@jetson 'bash -s' < detect.sh   # 远程执行

set -u

# ---------- 原始事实采集 ----------
ARCH="$(uname -m)"
HOSTNAME="$(hostname 2>/dev/null || echo unknown)"
UBUNTU_ID="$(grep -E '^ID=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '"')"
UBUNTU_VERSION_ID="$(grep -E '^VERSION_ID=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '"')"
PRETTY_NAME="$(grep -E '^PRETTY_NAME=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '"')"

NV_TEGRA=""
[ -f /etc/nv_tegra_release ] && NV_TEGRA="$(cat /etc/nv_tegra_release)"

L4T_CORE_VER=""
if dpkg-query -W -f='${Version}' nvidia-l4t-core >/dev/null 2>&1; then
  L4T_CORE_VER="$(dpkg-query -W -f='${Version}' nvidia-l4t-core 2>/dev/null)"
fi

JETPACK_DPKG_VER=""
if dpkg-query -W -f='${Version}' nvidia-jetpack >/dev/null 2>&1; then
  JETPACK_DPKG_VER="$(dpkg-query -W -f='${Version}' nvidia-jetpack 2>/dev/null)"
fi

MACHINE_ID=""
[ -f /etc/machine-id ] && MACHINE_ID="$(tr -d ' \t\r\n' < /etc/machine-id 2>/dev/null)"

# 设备树出厂序列号：Jetson 模组唯一。真机发现克隆镜像的 /etc/machine-id
# 会重复（两台板同值），serial-number 才是每板唯一的稳定 ID 首选。
SERIAL_NUMBER=""
[ -f /proc/device-tree/serial-number ] && SERIAL_NUMBER="$(tr -d '\0 \t\r\n' < /proc/device-tree/serial-number 2>/dev/null)"

# 所有全局 IPv4（候选路径）；格式 "iface=addr"，过滤/分类在 python 侧完成
# （只读，不装任何东西）。
IPV4_ALL=""
if command -v ip >/dev/null 2>&1; then
  IPV4_ALL="$(ip -4 -o addr show scope global 2>/dev/null | awk '{split($4,a,"/"); print $2"="a[1]}' | tr '\n' ' ')"
fi
if [ -z "$IPV4_ALL" ] && command -v hostname >/dev/null 2>&1; then
  # 兜底：无接口名，按 "=addr" 处理（python 侧跳过接口名过滤）
  IPV4_ALL="$(hostname -I 2>/dev/null | tr ' ' '\n' | sed 's/^/=//' | tr '\n' ' ')"
fi

MODEL=""
[ -f /proc/device-tree/model ] && MODEL="$(tr -d '\0' < /proc/device-tree/model 2>/dev/null)"

COMPATIBLE=""
[ -f /proc/device-tree/compatible ] && COMPATIBLE="$(tr -d '\0' < /proc/device-tree/compatible 2>/dev/null)"

# ---------- 派生字段 ----------
# is_jetson：有 nv_tegra_release 或 设备树含 nvidia / l4t-core 已装，三者任一即可。
IS_JETSON="false"
if [ -n "$NV_TEGRA" ] || [ -n "$L4T_CORE_VER" ] \
   || echo "$COMPATIBLE $MODEL" | grep -qi 'nvidia'; then
  IS_JETSON="true"
fi

# L4T 版本：从 nv_tegra_release 提取 R3X.Y（如 "# R36 (release)"）
L4T_VERSION=""
if [ -n "$NV_TEGRA" ]; then
  L4T_VERSION="$(echo "$NV_TEGRA" | grep -oE 'R[0-9]+\.[0-9]+' | head -n1)"
fi

# JetPack 版本：优先 nvidia-jetpack 包版本；其次 L4T 主次映射；再其次镜像名启发式。
jetpack_from_l4t() {
  case "$1" in
    R39.*) echo "7.x" ;;
    R36.4*) echo "6.2" ;;
    R36.3*) echo "6.1" ;;
    R36.*)  echo "6.0" ;;
    R35.*) echo "5.x" ;;
    R34.*) echo "5.x" ;;
    R32.*) echo "4.x" ;;
    *) echo "" ;;
  esac
}
JETPACK_VERSION=""
if [ -n "$JETPACK_DPKG_VER" ]; then
  JETPACK_VERSION="$JETPACK_DPKG_VER"
elif [ -n "$L4T_VERSION" ]; then
  JETPACK_VERSION="$(jetpack_from_l4t "$L4T_VERSION")"
fi
# Seeed 镜像名形如 recomputer-mini-agx-orin-64g-j501-6.2.1-36.4.4-...，兜底提取 6.2.1 这类主次修订号
if [ -z "$JETPACK_VERSION" ] && [ -n "$NV_TEGRA" ]; then
  JETPACK_VERSION="$(echo "$NV_TEGRA" | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?-[0-9]+\.[0-9]+' | head -n1 | grep -oE '^[0-9]+\.[0-9]+(\.[0-9]+)?')"
fi

# ---------- 远程桌面组件现状（Phase 3 的 check 也会复用这些字段） ----------
pkg_installed() { dpkg-query -W -f='${Status}' "$1" >/dev/null 2>&1 && dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q 'install ok installed'; }

XRDP_INSTALLED="false";   pkg_installed xrdp     && XRDP_INSTALLED="true"
XORGXRDP_INSTALLED="false"; pkg_installed xorgxrdp && XORGXRDP_INSTALLED="true"
XFCE_INSTALLED="false";   pkg_installed xfce4    && XFCE_INSTALLED="true"

XRDP_ACTIVE="false"
systemctl is-active --quiet xrdp 2>/dev/null && XRDP_ACTIVE="true"

PORT_3389="closed"
if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ':3389 '; then
  PORT_3389="listening"
fi

# ---------- 输出 JSON（python3 保证转义，值经 env 传入避免 arg 换行/长度问题） ----------
export ARCH HOSTNAME UBUNTU_ID UBUNTU_VERSION_ID PRETTY_NAME \
       NV_TEGRA L4T_CORE_VER JETPACK_DPKG_VER MODEL COMPATIBLE \
       IS_JETSON L4T_VERSION JETPACK_VERSION MACHINE_ID SERIAL_NUMBER IPV4_ALL \
       XRDP_INSTALLED XORGXRDP_INSTALLED XFCE_INSTALLED \
       XRDP_ACTIVE PORT_3389

python3 - <<'PY'
import json, os
def g(k):
    return os.environ.get(k, "")
def classify_ips(raw):
    import ipaddress, re
    # 虚拟/桥接接口一律不作为可达路径：docker、L4T USB 桥（Seeed 用
    # 192.168.56 网段）、libvirt、VM 主机网络等。
    skip_iface = re.compile(
        r"^(lo|docker|br-|l4tbr|virbr|veth|usb|lxcbr|vmnet|vboxnet)"
    )
    out = []
    for tok in raw.split():
        iface, _, addr = tok.partition("=")
        if not addr:
            addr, iface = tok, ""
        if iface and skip_iface.match(iface):
            continue
        try:
            ip = ipaddress.ip_address(addr)
        except ValueError:
            continue
        if ip.version != 4 or ip.is_loopback or ip.is_link_local:
            continue
        if ip in ipaddress.ip_network("172.17.0.0/16"):
            continue
        if ip in ipaddress.ip_network("192.168.55.0/24"):
            continue
        if ip in ipaddress.ip_network("100.64.0.0/10"):
            out.append({"address": str(ip), "kind": "tailscale"})
            continue
        if ip.is_private:
            out.append({"address": str(ip), "kind": "lan"})
    return out
d = {
    "is_jetson": g("IS_JETSON") == "true",
    "architecture": g("ARCH"),
    "hostname": g("HOSTNAME"),
    "ubuntu_id": g("UBUNTU_ID"),
    "ubuntu_version": g("UBUNTU_VERSION_ID"),
    "pretty_name": g("PRETTY_NAME"),
    "l4t_version": g("L4T_VERSION"),
    "jetpack_version": g("JETPACK_VERSION"),
    "device_model": (g("MODEL") or "").strip(),
    "machine_id": g("MACHINE_ID"),
    "serial_number": g("SERIAL_NUMBER"),
    "ipv4_addresses": classify_ips(g("IPV4_ALL")),
    "nvidia_l4t_core_default_version": g("L4T_CORE_VER"),
    "nv_tegra_release": g("NV_TEGRA").strip(),
    "remote_desktop": {
        "xrdp_installed": g("XRDP_INSTALLED") == "true",
        "xorgxrdp_installed": g("XORGXRDP_INSTALLED") == "true",
        "xfce_installed": g("XFCE_INSTALLED") == "true",
        "xrdp_active": g("XRDP_ACTIVE") == "true",
        "port_3389": g("PORT_3389"),
    },
}
print(json.dumps(d, ensure_ascii=False, indent=2))
PY