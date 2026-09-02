# TESTING_JETSON — 真机测试清单

> 目标设备：`seeed@192.168.100.164`（reComputer J501 mini，JetPack 6.2.1 / Ubuntu 22.04）。
> 完成度：Phase 0 已完成 Test 2/6/9 的核心子集；完整电子化（App 一键）留到 Phase 2–6。

## 环境准备
- 一台干净 Jetson（无 HDMI），SSH 可达，sudo 用户。
- Mac 上 `sdl-freerdp`（brew `freerdp`）。

## Test 清单

- [x] **Test 1 · 首次安装（无 HDMI）** — Phase 0 真机完成：bootstrap 装 xrdp/xorgxrdp/xfce4，Xorg :10 + XFCE 拉起。（App 化待 Phase 3）
- [x] **Test 2 · 第二次连接** — 重复 bootstrap 幂等（`already_installed` 短路）；再次 FreeRDP 连接成功。
- [x] **Test 3 · 断开重新连接** — 已验证：断开后 session 持久，重连 `reconnected session :10.0` 复用同一 pid。
- [ ] **Test 4 · Jetson reboot** — 待：重启后 `systemctl is-active xrdp` 仍 active（enabled 已设）。
- [ ] **Test 5 · 错误密码** — 待：连接失败且无密码泄露（拒绝对 `/p:` argv 的回归）。
- [ ] **Test 6 · 错误 IP** — 已部分：`3389 CLOSED`（未装时）；App 层 SSH 超时→错误提示待 Phase 2。
- [ ] **Test 7 · 非 Jetson Linux** — 待：detect.sh `is_jetson=false` 分支（可对任意 x86 VM 跑 detect.sh）。
- [ ] **Test 8 · XRDP service stopped** — 待：`systemctl stop xrdp` 后连接失败并给出可诊断错误。
- [x] **Test 9 · 重复 bootstrap** — 已完成：幂等验证通过。

## 手动复现命令

```bash
# 1. 上传并跑检测
scp scripts/remote/detect.sh seeed@192.168.100.164:/tmp/ && ssh seeed@192.168.100.164 'bash /tmp/detect.sh'

# 2. 上传并跑 bootstrap（sudo 密码经 stdin）
scp scripts/remote/bootstrap.sh seeed@192.168.100.164:/tmp/
printf 'PASSWORD\n' | ssh seeed@192.168.100.164 'bash /tmp/bootstrap.sh'

# 3. Mac 连接（原生窗口验证视觉）
printf 'PASSWORD\n' | sdl-freerdp /v:192.168.100.164 /u:seeed /from-stdin /cert:ignore

# headless 自动化验证（无窗口）
printf 'PASSWORD\n' | SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy sdl-freerdp /v:192.168.100.164 /u:seeed /from-stdin /cert:ignore
```

## Phase 2 真机测试清单（SSH 探测链路）✅

> 后端语义已用 `probe_verify` harness 在真机验证 **5/5 通过**（2026-08-31）；指纹 `SHA256:Ff2cXXUQLiD4y6CwJdqxxc5HQNARwTWE01WpAOyUBiI`。
> 前端 store 状态机已单测覆盖；App 已确认可 launch。**UI 视觉点击流程**（`cargo tauri dev` + `Mode=Real`）需人工最后确认一次。

- [x] Test 1 · 正确凭据 → 设备信息：NVIDIA Jetson AGX Orin DevKit / JetPack 6.2.1+b38 / Ubuntu 22.04 / L4T R36.4 / aarch64
- [x] Test 2 · 错误密码 → AuthenticationFailed
- [x] Test 3 · 错误 IP → 不可达（超时/拒绝）
- [x] Test 4 · 首次连接 → HostKeyUnknown（`ssh-ed25519 SHA256:Ff2c…`）
- [x] Test 5 · Trust → 重连成功进 detect
- [x] Test 6 · 篡改存指纹 → HostKeyChanged（prev≠cur）
- [x] Test 7 · 连接中 Cancel 不回弹（store 单测覆盖）

复现：
```bash
printf 'PASSWORD\n' | cargo run --bin probe_verify -- 192.168.100.164 seeed
```

## Phase 3 真机配置测试清单（自动 Provisioning）✅

> 由 `provision_probe` harness 真机验证（2026-08-31），不 purge、仅安全 repair。

- [x] Ready 快路径：`check` → `Ready` → **不执行 bootstrap**
- [x] 安全 repair：`systemctl stop xrdp` → `check`=`Broken`(xrdp not running + 3389 not listening) → `provision`（流式 Installing→Configuring→Starting→Verifying）→ `verify`=`Ready`
- [x] 幂等：repair 后再跑 → `Ready` 快路径、不重复 provision
- [x] sudo 密码经 stdin（`sudo -S -p ''`），未进 argv/log
- [ ] 完整 fresh 安装（apt 装包）：需干净设备/镜像；由 MockRemoteExecutor + Phase 0 已证 bootstrap 幂等覆盖

复现：
```bash
printf 'PASSWORD\n' | cargo run --bin provision_probe -- 192.168.100.164 seeed
```

## Phase 4A 真机测试清单（FreeRDP Sidecar）✅（传输层）

> 由本机 `sdl-freerdp 3.31.0`（brew arm64）headless 验证（2026-09-01）；用 `RdpProcess` 精确构造的同款 argv/stdin。

- [x] `/args-from:stdin` 契约：一参一行、argv 仅 `sdl-freerdp /args-from:stdin`（`--help` + dummy 连接到达 framebuffer 确证）
- [x] `/cert:tofu` 静默接受证书（无交互、无 cert 错误）
- [x] 正确凭据 `/p:seeed` → 真机 `reconnected session: username seeed, display :10.0`（sesman 日志确证 XFCE session 复用）
- [x] SSH 凭据 `seeed` 有效、环境 Ready（`probe_verify` 6/6 PASS）
- [x] SIGTERM → FreeRDP 干净 teardown（`ERRCONNECT_CONNECT_CANCELLED`），远端 Xorg :10 / xfce4-session 存活
- [x] `license BB_ERROR_BLOB` 判定为非致命 XRDP 杂讯（KI-011）

复现（headless，无窗口）：
```bash
printf '/v:192.168.100.164:3389\n/u:seeed\n/p:PASSWORD\n/cert:tofu\n+clipboard\n+dynamic-resolution\n' \
  | SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy sdl-freerdp /args-from:stdin
```

**需用户亲眼确认（原生窗口，无法 headless 自动化）**：
- `cargo tauri dev` → 输入 `192.168.100.164 / seeed / seeed` → Connect → Trust → 自动弹出 XFCE 桌面窗口。
- 键盘 / 鼠标 / 文字剪贴板双向 / resize 动态分辨率 / Retina 缩放对齐。
- macOS 本地网络权限是否拦截未签名 Homebrew FreeRDP（KI-004）。