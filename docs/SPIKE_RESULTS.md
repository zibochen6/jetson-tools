# SPIKE_RESULTS — Phase 0 Technical Spike

> Phase 0 目标：证明 `headless Jetson → XRDP + xorgxrdp → XFCE → FreeRDP → macOS` 完整路径可行，不做 UI。
> 结论：**✅ 完整闭环已在真机验证通过**。

## 1. 实验环境

| 项 | 值 |
|---|---|
| 客户端 | MacBook（macOS 27.0 arm64），FreeRDP 3.31.0（brew） |
| 设备 | `seeed@192.168.100.164`（reComputer J501 mini / AGX Orin 64G） |
| 系统 | Ubuntu 22.04.5，JetPack 6.2.1+b38（L4T R36.4.4）——属 PRD 的 **P1**（6.x/22.04），非 P0（7.x/24.04） |
| 初始态 | 无 HDMI（`XDG_SESSION_TYPE=tty`）、xrdp/xorgxrdp/xfce4 均未装、sudo 需密码、apt 源=中科大 `mirror.sysu.edu.cn` |

## 2. 执行的步骤与结果

| # | 步骤 | 结果 |
|---|---|---|
| 1 | `scripts/remote/detect.sh` 真机运行 | ✅ 输出结构化 JSON，正确识别 is_jetson/l4t/jetpack/arch/组件现状 |
| 2 | `scripts/remote/bootstrap.sh` 真机运行（sudo 密码经 `sudo -S` stdin） | ✅ 安装 xrdp 0.9.17 + xorgxrdp 0.2.17 + xfce4 4.16，写 `~/.xsession`，服务 active、3389 listening |
| 3 | FreeRDP 连接（`sdl-freerdp`，SDL dummy 无窗口，`/from-stdin` 传密码） | ✅ 连接建立，登录成功 |
| 4 | XFCE 会话拉起 | ✅ Xorg `:10` + xfce4-session + xfwm4 + xfce4-panel + xfdesktop 全运行 |
| 5 | 断开（杀客户端） | ✅ session 持久（Xorg :10 + xfce4-session 存活，sesman 无 terminated） |
| 6 | 重连 | ✅ `reconnected session: display :10.0`，**复用同一** Xorg(pid 12391)/xfce4-session(pid 12495)，未新建 :11 |
| 7 | 重复 bootstrap | ✅ 幂等（`already_installed` / `already active` 短路，`WaylandEnable already false`） |

## 3. 关键发现（踩坑记录）

### F1（关键/阻断）— NVIDIA `.xsessionrc` bash 数组在 dash 下中断 Xsession，导致 XFCE 会话 0 秒退出
- **现象**：首次连接后 Xorg :10 正常启动、登录成功，但 `startwm.sh` → `/etc/X11/Xsession` 立刻 `exit 2`，`~/.xsession` 里的 `startxfce4` 根本没执行到。
- **根因**：`/etc/X11/Xsession` 用 `sh`(dash) 运行，其 `40x11-common_xsessionrc` 会 source `~/.xsessionrc`。JetPack 镜像自带的 `.xsessionrc` 第 85 行是 bash 数组赋值 `remove_apps=("a" "b" "c")`，dash 报 `"(" unexpected`，整条 Xsession 链中断。
- **影响范围**：所有 JetPack 设备（`.xsessionrc` 随 `/etc/skel` 下发）。GDM 的 GNOME 会话不走 Xsession 所以物理登录不受影响，唯独 xrdp/XFCE 命中。
- **修复**：`bootstrap.sh` 步骤 3b —— 若 `sh -n ~/.xsessionrc` 失败则重命名为 `.xsessionrc.disabled-by-jetson-remote`（幂等、可逆、保留原件）。
- **诊断线索**：`~/.xsession-errors` 有 `Xsession: 85: .xsessionrc: Syntax error`；`/var/log/xrdp-sesman.log` 有 `Window manager exited with non-zero exit code 2 (0 secs)`。

### F2 — FreeRDP `/from-stdin` 只在完整连接生效，`+auth-only` 不读 stdin
- `+auth-only` 模式在 preConnect 就报 `auth-only, but no password set`（不读 stdin）。
- **完整连接**时 `/from-stdin` 正常从 stdin 读密码（本次登录成功验证）。作为产品凭据通道 OK。

### F3 — brew FreeRDP 3.31 的 SDL 客户端二进制名是 `sdl-freerdp`
- 不是文献常写的 `sdl2-freerdp`/`sdl3-freerdp`（brew 用 `WITH_CLIENT_SDL_VERSIONED=OFF`）。
- `xfreerdp` 是 X11 程序，macOS 需 XQuartz；**用 `sdl-freerdp`（SDL native，无需 XQuartz）**。

### F4 — SDL dummy 驱动可实现无窗口 headless 客户端测试
- `SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy` 下 `sdl-freerdp` 可完整建 session（软件渲染），便于自动化验证；产品正式运行时不设 dummy（弹原生窗口）。

### F5 — `device_model` 来自设备树是通用名
- `/proc/device-tree/model` = "NVIDIA Jetson AGX Orin Developer Kit"（通用），实际板卡是 Seeed reComputer J501 mini（型号在 `/etc/nv_tegra_release` 的镜像名 `...-j501-6.2.1-...`）。MVP 用通用名已够；如需精确 carrier 型号再解析镜像名。

### F6 — 网络环境：官方源 IP 是 Clash fake-ip，apt 走中国镜像
- `archive.ubuntu.com` 解析到 `198.18.1.69`（fake-ip 段）；实际 apt 源是 `mirror.sysu.edu.cn`（可达）。产品/diagnostic 不要假设 apt 源域名。

### F7 — sudo 非免密，用 `sudo -S -p ''` 从 stdin 读密码一次、管道复用给多次 sudo
- `bootstrap.sh` 的 `as_root()` 已封装：root 直跑 / `sudo -n` / `sudo -S` stdin 三态。

## 4. 已验证 / 未验证

**已验证**：核心 remote workflow（检测、安装、配置、连接、断线重连复用 session）、幂等、`/from-stdin` 凭据、SDL 无窗口测试。
**未验证（需用户侧/真机 GUI）**：
- 苹果本地网络权限是否拦截未签名 `sdl-freerdp`（本次节点可能已授权或走已允许网络）。
- FreeRDP **原生窗口**的实际视觉体验（用户需在 Mac 上亲眼确认桌面）。命令：
  ```bash
  printf 'PASSWORD\n' | sdl-freerdp /v:192.168.100.164 /u:seeed /from-stdin /cert:ignore
  ```

## 5. 对后续 Phase 的输入

- Phase 3 产品化：`bootstrap.sh` 已含 3b 自愈；需把 `[bootstrap] step=*` 输出流式喂给前端进度。
- Phase 4：FreeRDP sidecar 用 `sdl-freerdp` + `/from-stdin`，argv 不含密码。
- Phase 5：Jetson XRDP 当前监听 `*:3389`，切隧道时改成 `127.0.0.1`（改 `/etc/xrdp/xrdp.ini` `port` 前或加 `listen` 选项）。