# KNOWN_ISSUES — Jetson Remote

> 持续维护。记录已知坑、根因、缓解/修复、状态。

## KI-001 — NVIDIA `.xsessionrc` bash 语法中断 xrdp/XFCE 会话（阻断级，已修复）
- **现象**：xrdp 连接后 Xorg :10 正常、登录成功，但会话 0 秒退出（`~/.xsession-errors`: `Xsession: 85: .xsessionrc: Syntax error: "(" unexpected`；sesman: `Window manager exited ... exit code 2`）。
- **根因**：JetPack 镜像自带 `~/.xsessionrc` 含 bash 数组（`arr=("a" "b")`），被 `/etc/X11/Xsession`（dash/sh）source 时报错，中断链条 → `startxfce4` 未执行。
- **影响**：所有 JetPack 设备；仅 xrdp/XFCE 命中（GDM/GNOME 不走 Xsession）。
- **缓解**：`bootstrap.sh` 3b 步骤：`sh -n` 失败则重命名 `~/.xsessionrc.disabled-by-jetson-remote`。
- **状态**：已在真机修复并重测通过（SPIKE_RESULTS F1）。

## KI-002 — FreeRDP `+auth-only` 不读 stdin 密码
- **现象**：`+auth-only` preConnect 报 `no password set`。
- **根因**：auth-only 特殊路径不在登录阶段读 stdin。
- **影响**：仅影响用 auth-only 做连通测试；完整连接 `/from-stdin` 正常。
- **状态**：已知，规避（测试用完整连接 / 产品用 `/from-stdin` 不改）。

## KI-003 — `device_model` 设备树名为通用名
- **现象**：`/proc/device-tree/model` 返回 "NVIDIA Jetson AGX Orin Developer Kit"，非具体 carrier（如 Seeed reComputer J501 mini）。
- **缓解**：精确 carrier 型号可解析 `/etc/nv_tegra_release` 镜像名（含 `-j501-` 等），MVP 暂不需要。
- **状态**：低优先。

## KI-004 — macOS 本地网络(TCC)静默拒绝无 Team 签名的二进制（已确证，P0）
- **现象**：app 自身（russh 控制面）连不上 Jetson（`No route to host`），而系统 Terminal `ssh`/`nc` 正常；app **无任何「本地网络」授权弹窗**。
- **确证（2026-09-01，5 组对照）**：
  - Apple 签名 `nc`/`ssh` → `192.168.100.164:22`/`3389` **成功**。
  - 未签名 cargo 二进制（`network_probe`）→ `connect` 失败 `errno 65 EHOSTUNREACH`。
  - `cargo tauri build --debug` 的 .app（linker-signed）→ 失败、无弹窗。
  - `codesign --force --deep --sign - --identifier com.jetsonremote.app`（正确的 ad-hoc + 绑定 Info.plist + NSLocalNetworkUsageDescription，`codesign --verify` 通过）→ **仍失败、仍无弹窗**。
  - 全新 bundle id `com.jetsonremote.app.tccdebug1`（排除陈旧 TCC/LS 身份态）→ **仍失败、无弹窗**。
- **根因**：macOS 15(Sequoia)+ 的「本地网络」隐私授权**要求带 Team ID 的代码签名**（Apple Development / Developer ID）。ad-hoc / 无签名二进制被**静默拒绝**（`EHOSTUNREACH`，无 SYN 出网、无弹窗），仅 Apple 平台签名二进制放行。**不是 russh / RDP / 网络的问题**，是 app 身份在平台层拿不到本地网络授权。
- **影响**：正式签名前，app 直连局域网整条链路（SSH 控制面 + RDP）被 TCC 阻断；dev/自编译二进制始终命中。
- **候选路线**（待定）：① 正式签名（免费 Personal Team → Apple Development 证书，有 Team ID，弹窗正常、直连回归）；② 控制面改走系统 `ssh`（Apple 签名、免弹窗、隧道绕过 TCC，已有原型）。
- **缓解（dev，已实现，2026-09-01）**：dev tunnel 模式——设置 `VITE_JR_SSH_PORT`（可选 `VITE_JR_RDP_PORT`，默认 3389）后，SSH 控制面与 RDP 面一律路由到 `127.0.0.1` 回环隧道，用户输入的 LAN IP 仅保留为设备标识/展示；底栏显示 `TUNNEL 127.0.0.1:<ssh> / 127.0.0.1:<rdp>` 徽标。配合 `ssh -L 2222:localhost:22 -L 3389:localhost:3389` 隧道，未签名 dev 二进制即可全链路连通（真机验证：SSH 握手/检测、embedded RDP 重连 :10 均通过；首连 127.0.0.1:<port> 会触发 TOFU 信任提示，属预期）。
- **诊断工具**：`cargo run --bin network_probe -- <host> <port>`（原样透传 errno）；dev-only GUI 触发见 `DevNetworkProbe.tsx`。
- **状态**：开放（P0 平台限制；0.2.1 起 release 由 app 自建环回隧道绕过（KI-021），不再依赖手动隧道；直连路线仍待 Team ID 签名）。
- **更新（2026-09-08）**：KI-021 的「子进程 ssh 绕过」在 macOS 15+/27 同样失效（responsible-process 归属，见 KI-039）；候选路线①（免费 Personal Team 正式签名）已落地并真机验证通过（KI-039）。

## KI-005 — apt 源域名受网络环境 fake-ip 影响
- **现象**：`archive.ubuntu.com` 解析到 `198.18.1.69`（Clash fake-ip）。
- **缓解**：bootstrap 不硬编码官方源；DNS/连通失败时给出明确诊断，不假设域名可达。
- **状态**：记录。

## KI-006 — xrdp 监听 `*:3389`（正式版需绑 localhost）
- **现象**：当前 `*:3389` LAN 可达。
- **缓解**：Phase 5 隧道前改 `xrdp.ini` 只绑 `127.0.0.1`。
- **状态**：Phase 5 处理。

## KI-007 — pnpm 11 `ERR_PNPM_IGNORED_BUILDS` 阻断 `pnpm run`
- **现象**：`pnpm run`（含 `typecheck/test/build`）前的依赖检查会触发内部 `pnpm install`，因 esbuild 构建脚本被默认拦截而 `exit 1`。
- **缓解**：`pnpm-workspace.yaml` 加 `allowBuilds: { esbuild: true }`（pnpm 11 新 schema；旧 `pnpm.onlyBuiltDependencies` 字段已废弃）。
- **状态**：已修复。

## KI-008 — SSH host 证书不支持
- **现象**：russh 0.63 `check_server_key` 传 `PublicKeyOrCertificate`；Host 证书变体无 `fingerprint` 方法。
- **缓解**：证书变体标 `SHA256:unknown`（不静默信任）；OpenSSH sshd 默认从不以 host 证书面对客户端，实际不触发。
- **状态**：低优先，MVP 不做。

## KI-009 — Phase 2 真机验证设备离线
- **现象**：`192.168.100.164`（及 `.163`）ping 100% 丢包、`No route to host`（Mac 网关 6ms 正常）。设备曾断电/休眠。
- **影响**：一度阻塞 Phase 2 真机端到端验证。
- **缓解**：设备上电后已恢复，`probe_verify` 5/5 通过（见 docs/TESTING_JETSON.md）。
- **状态**：✅ 已解决。

## KI-010 — provisioning 期间关闭 App 可能中断远端 bootstrap
- **现象**：远端 bootstrap 依附 SSH channel；App 关闭 → channel 断开 → 远端 `sudo bash xxx.sh` 可能被 SIGHUP 终止，留下半配置态。
- **影响**：MVP 未做 window-close 拦截（成本权衡）。
- **缓解**：bootstrap 幂等，重跑 `prepare_remote_desktop` 会自愈到 Ready（idempotent repair）。
- **状态**：记录；Phase 6 考虑 window-close 确认。

## KI-011 — FreeRDP `license BB_ERROR_BLOB` 非致命杂讯
- **现象**：FreeRDP 3.31 连 XRDP 0.9.17 时 stderr 打印 `license binary blob::type expected BB_UNKNOWN, got BB_ERROR_BLOB`（含错误密码时也出现）。
- **根因**：XRDP 在 license 协商阶段回 error blob（「无 license server」信号），FreeRDP 记为错误但继续连接。
- **影响**：**不是**认证失败信号——真机确认同凭据最终 `reconnected session display :10.0` 成功。会进入 `RdpProcess` 诊断 ring buffer，但绝不上 UI。
- **状态**：已知非致命；诊断解析时需排除此误报。

## KI-012 — FreeRDP `/cert:tofu` 的 macOS 证书存储位置未在失败连接上落盘
- **现象**：`/cert:tofu` 接受证书无需交互，但本次失败-认证的连接未在 `~/.config/freerdp/` 或 `~/Library/Application Support/FreeRDP/` 观察到证书落盘。
- **影响**：TOFU 由 FreeRDP 自行管理（我们只传 `/cert:tofu`），跨连接一致性由 FreeRDP 保证；仅存储路径/生命周期待成功连接（带真实密码）观察确认。
- **状态**：记录；Phase 4A E2E 用了可逆 `/cert:tofu`，产品路径绝不 `/cert:ignore`。证书 store 非 app 命名空间（`~/.config/freerdp`）→ 未来 Phase 6 评估重定向。

## KI-013 — Embedded 桌面纯黑屏：RDPGFX 未接线（已修复）
- **现象**：Phase 4B-2 真机 Native View 已出现但纯黑；headless probe `EndPaint` 钩子**永不触发**（`on_frame`=0），`gdi_init_ex` 却正常打印 `Local/Remote framebuffer`。
- **根因（叙述于 2026-09-15 更正）**：`gdi_init()` 只接线 **legacy GDI** 路径（BitmapUpdate/绘图指令/SurfaceBits），**不注册 RDPGFX 图形管线**（`gdi_StartFrame/EndFrame/SurfaceCommand/CreateSurface/UpdateSurfaces`）。参考客户端在 RDPGFX 动态通道 connect 时懒调用 `gdi_graphics_pipeline_init`（`client/common/client.c:1610`），未订阅 PubSub 通道事件时该接线不发生。
  > **原文称「XRDP 默认协商 RDPGFX → 桌面经 GFX 通道下发」，该断言不成立。** xrdp 0.9.x（含两台目标设备上的 0.9.17 / 0.9.24）源码树里**没有 EGFX 模块**，无从协商 RDPGFX（`xrdp --version` 无 `--enable-x264`/`--enable-openh264`，无 `/etc/xrdp/gfx.toml`）。0.9.x 实际走 **legacy Fastpath Surface Commands + RemoteFX**——`gdi_graphics_pipeline_init()` 在 FreeRDP 侧同时接线这两条路径，所以**接线动作本身是对的**，只是被错误归因。
  > 真正让画面出现的是**那个无条件 33ms tick**；黑屏的原始机制（为何 `EndPaint` 不触发）**当时并未被证实**。完整证据见 KI-042。
- **影响**：所有需要 GFX / surface-commands 接线的桌面；`endpaint=0` 是典型信号（「连上了却收不到帧」）。
- **修复**：`bridge.c` `jr_pre_connect` 里 `PubSub_SubscribeChannelConnected(instance->context->pubSub, freerdp_client_OnChannelConnectedEventHandler)`（默认处理器内部已含 RDPGFX→gdi 接线），post_disconnect 反订阅。另因 GFX 更新不保证调用 legacy `EndPaint`，RDP event loop 每 33ms 提交一次已解码的 `primary_buffer`，使 native view 不会停在占位黑色。见 `EMBEDDED_RDP.md` §7。
  > **该修复保留，但改标为「防御性」**：对真正支持 GFX 的服务端（Windows / xrdp ≥ 0.10.2）它是唯一保护，且与 FreeRDP 参考客户端一致；对本项目的 xrdp 0.9.x 它不参与实际帧路径。
- **状态**：✅ 真机（SSH 隧道）视觉确认：XFCE 桌面与终端内容均可见（2026-09-02）。根因叙述于 2026-09-15 更正，见 KI-042。

## KI-014 — xrdp TLS 私钥权限：`xrdp` 用户在 `ssl-cert` 组外（自动修复）
- **现象**：某时刻起所有 RDP 连接在 X.224/CR-TPDU 阶段被掐断；设备日志 `Cannot read private key file /etc/xrdp/key.pem: Permission denied`。
- **根因**：`key.pem` 是 `→ /etc/ssl/private/ssl-cert-snakeoil.key` 符号链接；`/etc/ssl/private/` 为 `0710 root:ssl-cert`，而 `xrdp` 进程以 `xrdp` 用户(uid 114)运行，**不在空的 `ssl-cert` 组** → 读不到 TLS 私钥。
- **影响**：阻塞全部新连接（与客户端无关）。
- **修复**：`check-environment.sh` 读取 `id -nG xrdp` 并把缺失组成员关系分类为 broken；现有 prepare 流程随即运行 `bootstrap.sh`，后者确保 `ssl-cert` 已安装、幂等执行 `adduser xrdp ssl-cert`，并仅在成员关系改变时重启 xrdp。已真机执行并复验（xrdp.log 变为 `Using default X.509 key file`）。
- **状态**：✅ 自动检测与自愈（2026-09-02）。

## KI-015 — xrdp 新建会话窗口管理器段错误（exit 139，根因未定）
- **现象**：新建 Xorg 会话（例如 `:11`）窗口管理器 2 秒内 `exit 139`（SIGSEGV）；sesman `Window manager exited with exit code 139`。老会话（`:10`）不受影响。
- **A/B 已排除 GL 合成器（2026-09-01）**：合成器开（baseline `use_compositing=true`）时 `.xsession-errors` 有 `xfwm4: Unsupported GL renderer (llvmpipe)` 警告后段错误；`use_compositing=false` 关合成器后 **GL 警告消失但仍在 `Segmentation fault`**（仍 exit 139）。→ **llvmpipe/合成器不是根因**（用户预判正确）。
- **真根因**：未定。关合成器后崩溃无 GL、无 session-manager 报错，是 xfwm4 更底层的段错误。下一步用 gdb 分析 core dump / 查 `dmesg` / 核实依赖库版本（libcairo、libX11、libEGL、libgtk 等，疑似 Jetson 包更新致 ABI 不符）。
- **影响**：**新建**会话（首连新设备）会命中；重连既有 `:10` 无影响。4B-2 Gate（`:10` 重连）不受阻。
- **状态**：开放；A/B 已做、根因待定，不强行归因 compositor/llvmpipe。
## KI-016 — D-Bus 单实例应用窗口开往物理显示器（已修复）

- **症状**：远程桌面里点终端图标，`gnome-terminal` 窗口出现在接显示器的物理会话里，远程视图里没有；文件管理器等「碰巧没在物理侧运行」的应用则正常。
- **根因**：xrdp 会话未起私有 D-Bus 会话总线，`DBUS_SESSION_BUS_ADDRESS` 落到 systemd user bus（`/run/user/$UID/bus`），同一用户所有会话共享。gnome-terminal 等单实例应用把开窗请求交给总线上已有的 server 实例（`DISPLAY=:1`，物理侧）。
- **修复**：`~/.xsession` 改为 `eval $(dbus-launch --exit-with-session)` + `exec startxfce4`，每会话私有总线；`scripts/remote/bootstrap.sh` 同步（幂等，grep `dbus-launch`）。修复需注销重建 RDP 会话生效。
- **状态**：已修复（2026-09-01，待用户重连验证）。

## KI-017 — xfwm4 在 xorgxrdp 会话冻死：拖窗/最大化全失效（已修复）

- **症状**：远程桌面里窗口有标题栏但拖不动、最大化/最小化无反应；客户端区域点击/键盘正常；设备侧 `xdotool` 直接拖标题栏也无效（EWMH 消息也无响应）→ WM 进程活着但完全不处理事件。
- **根因**：xfwm4 的 GL 合成器在 xorgxrdp 虚拟显示上挂死（与 KI-015 同源嫌疑）。
- **修复**：`~/.xsession` 会话启动即 `xfconf-query -c xfwm4 -p /general/use_compositing -s false`（bootstrap.sh 幂等同步）；冻死时可用 `xfwm4 --replace` 救活。
- **伴生 UX 修复**：会话分辨率改为按窗口内容区宽高比登录（`session.rs desktop_size_for`），默认窗口横屏 1280x800——消除 letterbox（此前竖窗把 1280x720 压到 0.48 倍，标题栏屏上仅 ~14px，根本点不中）。
- **状态**：已修复（2026-09-01，真机验证：xorgxrdp 会话 1280x800、合成关、WM 存活、xdotool 拖拽生效）。

## KI-018 — Keychain 条目绑定 app 身份：未签名/dev 构建可能重复弹授权（已确认，低影响）

- **现象**：`cargo tauri dev` / ad-hoc 构建首次「记住设备」时，macOS 可能弹「jetson-remote 想要存取钥匙串中…」；重建二进制（身份变化）后可能再弹一次，用户需点「始终允许」。
- **根因**：Keychain generic-password 条目 ACL 绑定创建者 app 的 codesign identity；未签名/临时构建 identity 每次变化（与 KI-004 同源的 TCC 身份态问题）。
- **影响**：仅开发构建；正式签名包（`cargo tauri build` + 签名）身份稳定，不会重复弹。
- **缓解**：dev 下首次弹窗点「始终允许」；或改用签名构建。0.2.1 起密码存储改为 0600 文件（KI-020），Keychain 不再使用，本问题随之消失。
- **状态**：✅ 已由 KI-020 方案消除（0.2.1）。

## KI-018 — RDP 指针 release 从未送达 X：拖窗/点击紊乱（已修复）

- **症状**：远程桌面拖不动窗、点击时灵时不灵；设备侧 `xinput test` 抓取显示 `button press` 大量、`button release` 为 0（xdotool 因 press/release 完整而正常）。
- **根因**：RDP 指针事件的按钮位表示"当前按住状态"、`PTR_FLAGS_DOWN` 表示按下跳变；release 靠位 1→0 跳变。我们的 mouseUp 发的是 `MOVE|BUTTON1`（无 DOWN）= "仍按住"，xrdp 永远等不到 release，按钮状态卡死后 WM grab/点击全部紊乱。
- **修复**：mouseUp/rightMouseUp/otherMouseUp 改为清掉按钮位后发 move（产生 release 跳变）。
- **状态**：已修复（2026-09-01）。

## KI-019 — 剪贴板不互通（已修复，双向真机验证 2026-09-02）

- **症状**：Mac 复制的文本无法粘贴进远程（远程→Mac 同样不通）。
- **根因（四层，全部修复）**：
  1. **时序竞争**：`freerdp_channels_get_static_channel_interface` 在 PostConnect 时对 cliprdr 返回空（通道 OPEN 尚未完成）；`ChannelConnected` PubSub 事件虽有触发但时机不定 → 回调未接线，ServerCapabilities/MonitorReady 被静默丢弃。修复：事件回调 + `jr_clip_ensure()` 懒重试（Mac 0.5s 轮询责任人，确定性兜底）。
  2. **回调签名 cast 错误**：PubSub 回调的 `context` 是 `rdpContext*`（发布方传 `instance->context`），不是 `freerdp*`。此前 cast 错导致 session 指针为垃圾值、误判"已接线"。修复：按 `rdpContext*` 解引用（`jr_context_t*` 与 `rdpContext*` 地址相同）。
  3. **握手方向**：MS-RDPECLIP 由**客户端先发 ClientCapabilities**（参照 FreeRDP 官方 X11/SDL 客户端）；服务器回的 ServerCapabilities 只记录不回（避免 ping-pong）；MonitorReady 后宣告本地剪贴板。
  4. **线程安全**：`jr_mac_clip_get`（在 FreeRDP worker 线程被调用）改为主线程 dispatch_sync 读 NSPasteboard。
- **端到端验证**（xclip/pbcopy 闭环）：远程 `printf|xclip` → Mac `pbpaste` 得 `REMOTE_HELLO_888` ✓；Mac `printf|pbcopy` → 设备 `xclip -o` 得 `MAC_TO_REMOTE_777` ✓。
- **限制**：仅文本（CF_UNICODETEXT/CF_TEXT）；Mac→远程用 Ctrl+Shift+V，远程→Mac 用 Ctrl+Shift+C。
- **状态**：已修复并双向真机验证（2026-09-02）。

## KI-020 — ad-hoc 签名下 Keychain 每次启动弹授权密码（已修复，0.2.1）

- **症状**：app 一打开（含自动重连前）macOS 弹「想要使用钥匙串中的机密信息」，要求输入登录密码「允许/始终允许」；ad-hoc/未签名构建下「始终允许」无法持久生效，每次启动都弹。
- **根因**：V0.3「记住设备」把密码存 macOS Keychain；Keychain 条目 ACL 绑定创建者 app 的 codesign identity，ad-hoc 身份（无 Team ID）不被稳定信任 → 重复授权（与 KI-004 同源的平台身份问题）。
- **修复**：`remember.rs` 以 `FileSecretStore` 替换 `KeychainSecretStore`：密码存 app 配置目录 `secrets.json`（目录 0700、文件 0600，原子写）；移除 `keyring` 依赖。任何签名状态下都不再弹窗。
- **代价**：安全性从「Keychain ACL + 用户登录态」降为「POSIX 权限 + 用户登录态」；同机同用户进程可读。对本工具威胁模型可接受（等同 ~/.ssh 私钥文件的保护级别）。
- **迁移**：旧 Keychain 条目不自动迁移（迁移本身会触发弹窗）；升级后首次需重输一次密码并勾选记住。
- **状态**：✅ 已修复（0.2.1）。

## KI-021 — release 依赖手动 ssh 隧道，未开隧道即 “Couldn't reach this Jetson”（已修复，0.2.1）

- **症状**：v0.2.0 release 内嵌 `VITE_JR_SSH_PORT=2222` 路由，用户若没先在终端跑 `ssh -L 2222:localhost:22 -L 3389:localhost:3389 <user>@<jetson>`，app 连 127.0.0.1:2222 被拒 → 错误屏 “Couldn't reach this Jetson”。
- **根因**：KI-004 的缓解方案把「建隧道」留给了用户。
- **修复**：`tunnel.rs` — app 用系统 `/usr/bin/ssh`（Apple 签名、不受 TCC 限制）自建环回隧道：
  - 密码仅经 0700 目录内 `SSH_ASKPASS` 脚本传递（`SSH_ASKPASS_REQUIRE=force`，不进 argv）；
  - `-F /dev/null` + `accept-new` + 专用 known_hosts，不碰用户 ssh 配置；
  - 本地端口优先 2222/3389（主机密钥信任库按 wire host:port 计，端口稳定不反复 TOFU），被占则临时端口；
  - 端口开放后 3s 宽限检测 ssh 认证失败（`Permission denied` → auth_failed）；
  - app 退出时收掉隧道进程并删除凭据文件（RunEvent::Exit + Drop 双保险）。
  - 前端不再做端口路由，后端 probe/prepare/launch 统一走隧道；`VITE_JR_SSH_PORT` 编译期覆盖保留为 dev 复用外部隧道。
- **验证**：askpass/认证失败分类/accept-new 本机实测；0.2.1 真机全自动 E2E 通过（启动无弹窗 → 自建隧道 2222/3389 → 自动重连认证成功 → 桌面开启 → 剪贴板通道握手完成）；孤儿隧道启动清理实测。
- **状态**：✅ 已修复并真机验证（0.2.1）。


## KI-022 — 多设备（0.3.0）已知边界

- **剪贴板只跟焦点会话**：全局剪贴板同步同一时刻只绑定当前聚焦的桌面（bridge 全局单例）；切换 Tab 时自动迁移，后台会话不参与剪贴板。
- **会话分辨率在启动时固定**：每台设备的 RDP 分辨率按其启动时的窗口可视区（窗口高 − 44px Tab 栏）计算；之后改窗口大小走等比缩放（指针映射已补偿），重连才取新几何。
- **第 2/3 台设备用临时本地端口**：隧道优先端口 2222/3389 只给第一台，其余设备落临时端口 → 主机密钥信任库按 wire host:port 计，这些设备**每次 app 重启后可能重新 TOFU 弹一次信任确认**（第一台不受影响）。后续可改为每设备稳定端口分配。
- **侧车引擎不支持多会话**：`RDP_ENGINE=sidecar`（仅 dev）仍是单桌面；出货默认 embedded 引擎不受影响。
- **状态**：已知边界，非缺陷；真机回归建议见 0.3.0 release notes。

## KI-023 — 远端 Super 卡键：按 e 打开文件管理器、空格失灵（已修复）

- **症状**：连接 Jetson 后绝大多数键输入正常，唯独 **e 键一按就自动打开文件管理器（Thunar，Super+E）**、**空格键输入无效（被 Super+Space 输入法切换快捷键吃掉）**；重启 app、重连后依旧「一连接就这样」。
- **根因（两层叠加）**：
  1. app 把 Cmd 映射为 RDP Super（0x5B/0x5C, E0）。Cmd 的**松开** `flagsChanged` 若发生在 app 失活期间（Cmd+Tab 切应用、Cmd+Q 退出、Cmd+空格 Spotlight），macOS 不会把该事件投递给 app → 只有 Super down 到达远端、up 永远丢失 → 远端 Xorg 一直认为 Super 按住。
  2. xrdp 的 X 会话常驻复用（重连复用同一 Xorg :N），卡键状态跨 app 重启/重连持续 → 表现为「一连接就复现」。
- **修复（纯自动，无 UI）**：
  - `bridge.c` 新增 `jr_session_reset_keyboard_modifiers`（依次释放 LCtrl/RCtrl/LShift/RShift/LAlt/RAlt/LMeta/RMeta，释放未按下的键是服务端 no-op），`PostConnect` 每次连接/重连自动执行；
  - `macos_view.m`：窗口 resign/become key、app resign/become active、输入 attach（含多设备 Tab refocus）时触发同款修饰键 sweep 释放；`flagsChanged` 改为按 vk 建模 + 左右键共享 bit 时的 toggle 差分同步（避免漏发/错发），并新增 `[jr-input]` 键盘日志（keyDown/keyUp/flagsChanged/sweep）便于后续诊断。
  - 设计取舍：焦点恢复时**不**反向补发「仍按住」的修饰键——AppKit 修饰键 flag 无 L/R 区分，补发可能制造新卡键；下次真实 flagsChanged 自然重同步。
- **诊断口径（地面真值）**：`DISPLAY=:10.0 xinput query-state <键盘设备id>`（先 `xinput list` 找 id），空闲时 `down:` 列表含 Super_L(keycode 133) 即卡键确诊。
- **状态**：✅ 已修复（2026-09-02；真机回归见 CONNECTION_REGRESSION_GUIDE §3.1/§3.4）。

## KI-024 — 后台设备重连抢占前台桌面与隧道凭据目录共享（已修复，0.3.2）

- **症状**：同时连接多台 Jetson 时，后台设备断线后的自动重连会把用户正在操作的前台桌面切走；多条 SSH 隧道还共用同一份临时 askpass/password 目录，任一隧道销毁都会清理公共目录。
- **根因**：`launch_session` 只有“启动并聚焦”一种语义，前端自动重试成功后也无条件更新 `activeId`；隧道管理器虽按设备保存多个子进程，但临时凭据路径仍是单设备时代的固定路径。
- **修复**：IPC 增加兼容的 `focusOnLaunch` 选项，后台恢复以隐藏模式创建 Native View、RDP 会话和隧道，仅在该 Tab 仍被选中时重新聚焦；每条隧道改用独立 0700 目录和 0600 密码文件，启动时清理遗留目录，外部开发隧道拒绝绑定第二个设备身份。
- **进程清理**：系统 SSH 进入独立进程组，超时、退出和 app 关闭时终止整组并有界回收，避免 askpass 子进程持有 stderr 导致清理阻塞。
- **状态**：✅ 自动化回归已覆盖；双真机验证是 0.3.2 发布门槛。

## KI-025 — 多设备 Tab 被原生画布遮挡、第二台“已连接”但白屏（已修复，0.3.2）

- **症状**：连接多台设备后，顶部 Tab 只露出一小条、无法正常切换；第二台状态显示已连接，但桌面区域持续纯白。
- **根因（两项独立问题）**：
  1. Tauri 的 macOS content view 延伸到标题栏下方，原生 NSView 只预留了 44pt Tab 高度，未加 `safeAreaInsets.top`，所以覆盖了大部分 Web Tab。
  2. XRDP 的 TLS/RDP 握手先于 sesman 桌面建立完成；第二台的 `xrdp-sesman` 当时处于“进程 active、3350 listening、但不处理 xrdp 请求”的假健康状态。FreeRDP 得到永久纯白缓冲，旧逻辑仍立即返回 `Opened`。
- **修复**：原生画布与 RDP 分辨率统一扣除 macOS 顶部安全区 + 44pt Tab；会话启动改为后台等待真实非纯色首帧后再聚焦。首轮 18 秒仍无桌面时，通过既有 SSH 隧道联动重启 `xrdp-sesman`/`xrdp` 并重试一次；第二次仍失败则返回明确连接错误，不再显示假成功。远端检查和 bootstrap 同时验证两个服务及 3389/3350。
- **状态**：✅ 自动化回归已覆盖；双真机视觉/切换验证是 0.3.2 发布门槛。

## KI-026 — 克隆镜像的 `/etc/machine-id` 跨板重复（identity-v3 已规避）

- **症状**：两台测试 J501（robotics 32G / mini 64G）的 `/etc/machine-id` 完全相同（`5dbfb124…`），hostname 也同为 `seeed-desktop` —— 出厂镜像克隆所致。若以 machine-id 作设备身份，两块板会被并成同一台设备（无法双开、TOFU 反复弹「身份变更」）。
- **规避（identity-v3）**：`deviceId` 优先取 `/proc/device-tree/serial-number`（Jetson 模组出厂序列号，每板唯一、重装系统不变；真机验证 mini=`1421123007848`），无序列号才回退 machine-id，两者皆无回退 host 身份（legacy）。见 ADR-031。
- **附带**：detect.sh 的路径采集同时按接口名过滤虚拟网桥（`l4tbr0`/`docker*`/`br-*` 等）——真机上 Seeed 的 L4T USB 桥用 192.168.56.0/24（不是 55），仅靠网段过滤会漏。
- **状态**：✅ 已在 identity-v3 实现中规避；robotics 板的序列号唯一性待其网络恢复后复核（模组序列号为出厂烧录，风险极低）。

## KI-027 — AppKit 主线程直呼 FreeRDP API：剪贴板修好鼠标坏 / 鼠标修好剪贴板坏（已修复，0.3.3）

- **症状**：历史症状「修好剪贴板后拖不动 XFCE 窗口」「修好拖拽后剪贴板断」。表现为不可复现的输入紊乱、偶发卡死。
- **根因**：v0.3.2 及之前的线程模型——AppKit 主线程的 mouse/key 事件处理器直接调用 `freerdp_input_send_*`；NSTimer（主线程）直接调 `ClientFormatList`；而 FreeRDP worker 线程的 CLIPRDR 回调又 `dispatch_sync` 回主线程读 `NSPasteboard`。跨线程 FreeRDP 访问 + 主线程阻塞 + 锁反转的组合，任何一个时序都可能让 input/clipboard 互相破坏。
- **修复（单 owner 规则，0.3.3）**：新增每会话命令队列（`native/freerdp_bridge/queue.{c,h}`，纯 C/pthread、独立可测）。AppKit 主线程只 `enqueue`（mouse/key/unicode/剪贴板快照/resize/修饰键清扫）并 SetEvent 唤醒；FreeRDP worker 在事件循环里 drain 并**唯一地**调用 FreeRDP 输入/CLIPRDR API，wake event 追加进 `WaitForMultipleObjects`。队列保证：BUTTON/KEY 严格 FIFO、连续 MOVE 合并为最新、绝不跨按键边界合并、溢出时淘汰最老 MOVE 绝不丢按键配对。
- **Wake 事件坑**：WinPR `CreateEventA` 不支持 auto-reset（运行时日志可证）——必须 manual-reset + worker 每轮 `ResetEvent`。
- **状态**：✅ 已修复（FFI 队列测试全绿；真机 B 会话验证）。KI-018 鼠标拖拽契约保持不变并有队列级回归测试。

## KI-028 — 剪贴板 worker dispatch_sync 主线程 + 裸全局 g_clipSession 跨会话污染（已修复，0.3.3）

- **症状**：多设备下剪贴板偶发「A 的内容盖到 B」；worker 被 NSPasteboard 慢 I/O 阻塞，放大 KI-027 的输入紊乱。
- **根因**：`jr_mac_clip_get()`（worker）`dispatch_sync` 回主线程；`g_clipSession` 裸全局无 generation，A 的迟到回调在用户切到 B 后仍写 Mac 剪贴板。
- **修复（v0.3.3）**：Mac→远端改为主线程快照 → 入队 `LOCAL_CLIPBOARD_TEXT` → worker 存快照 + 发 `ClientFormatList`（初始快照由 `jr_clipboard_sync_start` 主线程直接线程安全写入，MonitorReady 即可宣告）；远端→Mac 改 `dispatch_async` + 主线程复验 `_Atomic` owner/generation（迟到回调丢弃）；`ServerFormatDataRequest` 只读 session 拥有的 `clip_text` 快照。
- **状态**：✅ 已修复（编解码 FFI 测试含中文/emoji 代理对全绿；真机 B 会话 CLIPRDR 握手实测通过）。

## KI-029 — macOS 中文 IME 无法输入（已修复，0.3.3）

- **症状**：Mac 拼音输入法在远程桌面内无法输入中文——keyDown 只走 scancode 映射，IME 组合/提交从未到达远端。
- **修复（v0.3.3）**：`JRView` 实现 `NSTextInputClient`（最小正确集）。普通文本键走 `interpretKeyEvents` → `insertText` 提交 → `enqueue_unicode_text` → worker 解码 UTF-8→UTF-16 码元（含代理对）逐码元 unicode press+release；组合期拼音原始字母**不转发**。导航/功能/小键盘与 Cmd/Ctrl 组合继续 scancode（同字符不双发）；KI-023 修饰键清扫全保留。
- **状态**：代码完成 + 单测全绿；**GUI 真机门槛（中文直输 + 候选窗）待用户验证**。

## KI-030 — 「Couldn't reach this Jetson」无法区分失败阶段 + 编译期外部隧道残留（已修复，0.3.3）

- **症状**：任何隧道/回环/认证失败都显示同一句错误；历史上 `VITE_JR_SSH_PORT` 编译进旧 binary 后 unset 不生效导致多设备路由错乱。
- **修复（v0.3.3）**：编译期 `option_env!` 路由删除，改运行时 `JR_EXTERNAL_SSH_PORT`（dev-only 显式单设备调试模式；release 永远 Internal 多隧道）；启动打印 `tunnel mode=`。`TunnelError` 细分 6 类 stable code + russh 回环单独 `LOOPBACK_SSH_FAILED`；前端优先展示后端 detail。`TunnelManager.ensure` 补脱敏 trace，key-vs-target 决策抽出 `classify_ensure` 纯函数加多设备单测。
- **真机 trace 实证（2026-09-04，docs/P0A_TRACE_*.md）**：多设备后端不变量全部 PASS——B 后连拿首选端口，A 掉线后 32 次重连每次规划新临时端口且从不 kill/replace B；两隧道进程与两份独立凭据目录并存。原始「第二台连不上」在本环境定位为**目标设备当时真实不可达**（非身份/端口合并 bug）。
- **状态**：✅ 已修复；新二进制可区分设备不可达 / 认证失败 / 本地端口 / ssh 退出 / 回环。

## KI-031 — bootstrap 不支持 Ubuntu 20.04 / JetPack 5.x（已修复，0.3.3）

- **症状**：JetPack 5.x（Ubuntu 20.04，如 Orin NX J401）provision 时 `exit 2 unsupported_os=20.04`。
- **修复**：`bootstrap.sh` 版本分支加入 `20.04`（xrdp 0.9.12 / xorgxrdp 0.2.12 / xfce4 4.14 均在 focal 源；配置/自愈逻辑与 22.04 一致）。
- **真机验证（100.72.12.62 / Orin NX J401 / JP 5.1.3）**：bootstrap 一次通过 → `ready=true`；`embedded_rdp_probe` 实测出真实桌面帧（1280x720、nnz=4096/4096、on_frame 持续刷新）。
- **状态**：✅ 已修复并真机验证。

## KI-032 — FreeRDP 3.31 证书 TOFU「接受」= 必须能写 ~/.config/freerdp/server/（排查结论，非 bug）

- **排查**：probe 曾报 `certificate not trusted` 且 `verify_cert` 已返回 1。对照 FreeRDP 3.31.0 源码：`accept_certificate=1` 时 `verification_status = certificate_store_save_data(...) ? 1 : -1` ——「接受并存储」要求**写盘** `~/.config/freerdp/server/<host>_<port>.pem`，写失败即回退为拒绝。该 probe 在受限沙箱写不了该目录导致误报；换可写目录后接受→存储→连接→出帧全部正常。
- **结论**：App 证书代码与 FreeRDP 3.31 语义一致，无需修改。多设备临时端口下每设备累积多条 `<host>_<port>.pem`（KI-022 已知边界）。
- **状态**：✅ 排查完毕，无代码变更（记录防复发误判）。

## KI-033 — 新设备（Ubuntu 24.04）provision 必败：sudo 票据跨 SSH channel 不生效（已修复，0.3.4）

- **症状**：连接全新 Jetson（SSH/认证/检测全部成功）后，安装远程桌面阶段必败：UI 报「远程桌面安装失败」，日志为 `ipc error ProvisionFailed: Remote desktop setup failed (exit Some(1))`，重试永远同样失败。
- **根因**：provisioner 旧流程「通道 1 `sudo -S -v` 预验证缓存票据 → 后续通道 5 `sudo -n bash <script>` 以 root 跑脚本」。但 russh 每次 `exec` 都开**新的 SSH channel**，而 Ubuntu 24.04 的 sudo 1.9.15（`Defaults use_pty` + 默认 tty 时间戳）把票据绑定在 channel 的 pty/会话上下文——**跨 channel 不生效**，`sudo -n` 报 `a password is required` 退出码 1（正是 App 日志里的 exit Some(1)）。老设备（20.04/22.04 旧版 sudo）恰好能跨 channel 复用票据，掩盖了此问题。
- **修复（0.3.4）**：改用 bootstrap.sh 自己文档化的远程契约——`bash <path>` 以登录用户身份跑脚本，sudo 密码经 stdin 喂给脚本（脚本内部自管升级：有可用票据/免密则 `sudo cmd`，否则读 stdin 一行密码逐次 `sudo -S`）。密码仍只走 SSH channel stdin，绝不进 argv/日志；preflight 预验证密码逻辑保留。
- **真机验证（2026-09-04，100.105.116.71 / AGX Orin / Ubuntu 24.04）**：用比 App 更严苛的「每步独立 SSH 连接」复现新流程：preflight → mktemp → upload → chmod → `bash + stdin 密码` → **`ready=true` 退出码 0**；随后 RDP 隧道验证真实出帧（on_frame 54→631 持续、nnz>0、中心像素 XFCE 灰）。新设备从裸机到可用桌面一次通过。
- **状态**：✅ 已修复并真机验证；回归测试覆盖（新 command 契约 + 密码经 stdin 断言）。

## KI-034 — IME 组合期非文本键绕过输入法：组合永久卡死、中英文都无法输入（已修复，0.3.5）

- **症状**：连接后无法打字。中文输入法的浮动组合框出现并卡住；切到英文也无济于事，什么字符都进不了终端。日志里只有 Return/Backspace 等非文本键的 scancode 记录，`IME commit` 从未出现。
- **根因**：v0.3.3 的 keyDown 路由把 Enter/Esc/Backspace/方向键/Tab 归为「非文本键」直接走远程 scancode。当拼音组合处于活动态（marked text）时，这些键恰恰是用户提交/取消/选词的手段——被绕过后 IME 组合永远无法结束（组合框卡死），后续所有键继续喏进卡住的组合会话（英文也打不进）。另：`selectedRange` 返回 `{NSNotFound,0}` 被多个 CJK 输入法视为「无可用客户端」，加剧卡死。
- **修复（0.3.5）**：
  1. **组合期间一切键走 IME**：`jrShouldUseTextInput` 先检查 `hasMarkedText`，为真时（含 Enter/Space/Backspace/方向键/Esc/Cmd 组合）全部 `interpretKeyEvents:`，IME 能正常提交/取消/导航候选。
  2. **keyDown/keyUp 对称**：按 vk 记录 `_imeKey[]` 标志，press 走了 IME 的键 release 绝不再发孤立 scancode（避免远端 release-无-press 失步）。
  3. **doCommandBySelector 实装**：组合结束后 IME 转发的命令（insertNewline:/moveLeft:/deleteBackward:/cancelOperation: 等 16 个）映射回对应 scancode（自含 press+release），保证「拼音 Enter 提交后终端命令也被执行」。
  4. **selectedRange 返回有效空选区** `{0,0}`（对齐 GLFW/SDL/Chromium 实践）；attach 分离时清除组合状态；新增路由/组合/提交/命令诊断日志。
- **真机验证（2026-09-04，100.72.12.62）**：CGEvent 合成键 + 设备侧 `xinput test` 地面真值：(a) 无组合态单字母直接 `IME commit→unicode sent→设备收到 key event`；(b) 合成组合（字母×2 + setMarkedText 逃代）后 **Enter 触发 `IME commit len=2 → unicode sent units=2`，设备收到两个字符的 press/release**（即原卡死场景）；(c) 无组合态 Enter 走 scancode，设备收到 Return(36)——三条路径全部符合设计。
- **状态**：✅ 已修复并真机验证。

## KI-035 — 关闭设备标签不会关闭桌面：原生视图永不卸载，最后一帧永久覆盖首页（已修复，0.3.6）

- **症状**：点标签上的「×」后，标签从标签栏消失了，但远程桌面画面一直卡在窗口里不动，既不消失也回不到设备总览首页，只能退出 App。
- **根因（两层）**：
  1. **原生视图从不卸载**：`RdpSession::shutdown()` 只做了 `attach_input(None)` + 剪贴板停止 + `jr_session_disconnect` + join，**从未调用 `remove_from_window()`**（只有 `unfocus()` 调了）。而 `JRView` 是窗口 contentView 的 subview，**superview 持有强引用**：`jr_view_destroy` 里的 `__bridge_transfer` + release 只减一个引用计数，视图既不会 dealloc 也不会离开视图层级 —— 于是最后一帧桌面图像永久留在屏幕上，盖住 webview 首页。
  2. **前端关闭最后一个标签时没有让出屏幕**：`sessionsStore.closeTab` 在被关标签是当前激活项时，只在「还有其它 running 会话」时才调 `gateway.focus(next)`；没有下一个会话时**只把 `activeId` 置 null，从不调 `focus(null)`**。后端 `focus_session(null)` → `hide_all()` → `unfocus()` 是唯一会卸载视图的兜底路径，因此这条路也断了。而 `gateway.close()` 是 fire-and-forget（异常被吞），失败时 UI 完全没有反馈。
- **修复（0.3.6）**：
  1. `shutdown()` 在阻塞式 disconnect/join **之前**先 `remove_from_window()`：视图立刻让出屏幕，即使传输层拆卸耗时，「×」也是即时生效；同时修好了重试路径 / `clear_exited` / legacy `close_remote_desktop` 等所有 teardown 路径。
  2. `jr_view_destroy` 在 release 前先 `[v removeFromSuperview]`（C 层兜底，任何 drop 路径都不可能把已挂载的视图留在屏幕上）。
  3. `closeTab` 在没有下一个可显示会话时显式 `void gateway.focus(null)`，与 `showOverview` 语义一致（关闭 IPC 慢/失败也能回到首页）。
  4. 新增诊断：`[jr-flow] close_session id=…`、`[jr-view] unmount wasMounted=0|1`。
- **真机验证（2026-09-04，100.72.12.62，真实非白屏桌面）**：按窗口 ID 截图（不受遮挡影响）+ CGEvent 精确点击（hover 变红自校验命中「×」）：
  - 修复前：桌面区域像素**几乎完全不变**（diff=0.235，均值 128→127.8），标签却已消失 —— 卡死复现；
  - 修复后：桌面区域**整体变为首页**（diff=131.31，均值 117.8/std 86.5 → 247.2/std 15.2），标签栏字形簇 36→15（会话标签消失，只剩总览）；日志依次出现 `close_session id=…` → `ERRCONNECT_CONNECT_CANCELLED` → **`unmount wasMounted=1`**。
  - 回归测试：`closeTab` 关掉唯一会话时必须 `focus:null`（**去掉修复该测试即失败**，已实测）；关闭后台标签不得干扰当前屏幕。
- **状态**：✅ 已修复并真机验证。
## KI-036 — Jetson 断电重启后反复 "Couldn't reach this Jetson"（已修复）

- **症状**：Jetson 重新上电/重启后，app 端每次连接与 Retry 都失败并停在「Couldn't reach this Jetson / Could not reach the device」；需等约 45 秒甚至重启 app 才能恢复。
- **根因（两项叠加）**：
  1. `TunnelManager::ensure` 的健康检查只验证「ssh 子进程存活 && 本地 -L 转发端口可 accept」。设备断电后 TCP 半开，ssh 要到 `ServerAliveInterval(15s) × CountMax(3)` ≈ 45s 才自杀；期间本地转发照样 accept（远端 channel 已死），于是每次连接都复用这条僵尸隧道，russh 经 127.0.0.1 拨号必然超时。
  2. 前端只有一次启动自动重连（`initRemembered`），失败即落错误页；会话内自动重连链对「relaunch 失败（设备不可达）」也只尝试一次就置 `error` 终止。
- **修复**：
  - `is_healthy` 升级为端到端检查：额外经本地 ssh 转发读取远端 daemon 的 `SSH-` 标识行（EOF/超时即判定隧道死亡，连接被废弃并重新 spawn）。探测连接是隧道上的独立 channel，不影响在途流量。
  - 新增 `TunnelManager::invalidate(device_key)`：`probe/prepare` 经隧道拨号失败且错误为 `SshTimeout`/`ConnectionRefused` 时立即废弃该设备隧道，下一次 ensure 必然 spawn 全新 ssh。
  - 前端 `RDP_CONNECTION_FAILED` 映射为独立的 `rdp_connection_failed` 错误码；`sessionsStore` 重连链统一为「3 次 2s 快窗口 + 5 次 10s 慢窗口（覆盖设备重启），上限 8 次」，等待期间 Tab 显示重连脉冲、冻结画面隐藏；`initRemembered` 对 `ssh_timeout`/`tunnel_failed` 以 15s×3 补试，用户交互或其它结果即停止。
- **状态**：✅ 单测覆盖（banner 探测、重连链节奏/封顶/恢复、启动窗口重试）；断电重启真机验证见回归指南 §3.7。

## KI-038 — sysctl 禁用 IPv6 抹掉 lo 的 ::1，xrdp 永远连不上 sesman（已修复，自愈）

- **症状**：设备重启后 app 全自动重连能走到 RDP 握手（TLS、凭据都成功），但桌面永远起不来；xrdp 日志反复 `Error connecting to sesman: 127.0.0.1 port: 3350`，sesman 日志零建会话记录。设备端 `xrdp`/`xrdp-sesman` 服务 active、3350/3389 都在监听——环境检查全绿但实际链路已死。**每次开机必现**（用户「每次重启就连不上」的设备侧根因）。
- **根因（三层叠加）**：
  1. `/etc/sysctl.conf` 手动设置 `net.ipv6.conf.all/default.disable_ipv6=1`（常见的"网络优化"），内核随之抹掉 lo 的 `::1/128`。
  2. xrdp 0.9.17 `connect_loopback`（os_calls.c）对 `127.0.0.1` 目标**先试 `::1`**：非阻塞 connect 得 `EINPROGRESS` 即返回"进行中"——lo 无 ::1 时该连接永不完成（内核还会把源地址选成全机唯一 v6 地址 = enp3s0 的 fe80 链路本地，日志里的 `Closed socket (AF_INET6 fe80::...)` 即此）。
  3. 后续重试在未完成的 connect 上得 `EALREADY`，v4 回退分支（127.0.0.1 / ::ffff:127.0.0.1）永远不执行 → xrdp 到 sesman 零包、永久失败。
- **诊断要点**：`ip -6 addr show lo` 无 `inet6 ::1/128`；`ss -tln6` 见 sesman 监听 `*`；python 非阻塞 `connect(::1)` 返回 EINPROGRESS 且 getsockname 为 fe80 即实锤。tcpdump lo 上 3350 零包（连接在进程内即失败）。
- **修复（自愈，0.3.7）**：
  - `check-environment.sh` 新增 `lo_ipv6_loopback` 事实；Rust `classify` 将其纳入 service_ok（缺失 → Broken → 走自愈装机）。
  - `bootstrap.sh` 新增幂等步骤：注释 sysctl 中的 disable_ipv6 行（带 KI-038 标记）+ 运行时恢复 + 兜底显式加回 `::1/128`。
- **状态**：✅ 真机（robotics 32G）验证：修复 lo 的 ::1 后桌面恢复；自愈逻辑单测覆盖（classify broken + ENV 字段）。

## KI-039 — macOS 本地网络 TCC 连「系统 ssh 子进程隧道」也拦：ad-hoc 签名 APP 直连 LAN 全链路被拒（已修复，正式签名）

- **症状（2026-09-08 真机）**：App 连 LAN IP（`192.168.2.20`）必现「Couldn't reach this Jetson / TUNNEL_TARGET_UNREACHABLE」；同一时刻终端 `ssh seeed@192.168.2.20` 正常；同一台设备走 Tailscale（`100.114.170.49`，非本地网络）App 也能连。`tunnel.rs` 的瞬时重试（2s/4s/8s×3）无效——**确定性拒绝，不是瞬时路由**。
- **根因（对 KI-004/KI-021 的更新认知）**：
  1. KI-021 的绕过（app spawn 系统 `/usr/bin/ssh` 做 LAN 腿）在 macOS 15+/27 上已失效：本地网络 TCC 按 **responsible process** 把 ssh **子进程**的连接归属回 App，Apple 签名的子进程不再免疫。
  2. 归属回 App 后，ad-hoc 签名 App 没有 `application-identifier` 授权身份（无 Team ID）→ `nehelper` 显式报 `not platform entitled and no application ID is set, permission denied` → connect() 得 `EHOSTUNREACH(errno 65)`，OpenSSH 表面话术即 **"No route to host"**。
- **实锤链条**：
  - `~/Library/Logs/jetson-remote.log`：`tunnel ssh exited Some(255): ssh: connect to host 192.168.2.20 port 22: No route to host`，重试 3 次逐行同错。
  - `log show --style compact --last 20m --predicate 'eventMessage CONTAINS "LocalNetwork"'`：`UserEventAgent: not platform entitled and no application ID is set, permission denied`。
  - `codesign -dv /Applications/Jetson Remote.app`：`Signature=adhoc` / `TeamIdentifier=not set` / `Info.plist=not bound`。
- **修复（2026-09-08 真机验证：隧道 `192.168.2.1→192.168.2.20:22 ESTABLISHED`，2222/3389 双平面通过）**：
  1. **正式签名（KI-004 候选路线①落地）**：Xcode → Settings → Apple Accounts 登录 Apple ID（免费 Personal Team，Team ID `9Y4B7WBUU2`）→ 生成 "Apple Development" 证书。
  2. **修证书信任链**：本机密钥链里的 `Apple Worldwide Developer Relations Certification Authority`（2013–2023）**已过期**，导致新证 `security find-identity -v` 报 0 valid、codesign 报 `unable to build chain`；从 Apple 下载安装 `AppleWWDRCAG3.cer`（2020–2030）后恢复 1 valid。
  3. **`tauri.conf.json`**：`bundle.macOS.signingIdentity` = 该证书 SHA-1；`entitlements` = `entitlements.plist`（内含 `com.apple.application-identifier = <TeamID>.com.jetsonremote.app`——nehelper 要的 "application ID" 就是它）；**`hardenedRuntime = false`（必须）**：开启 hardened runtime 会启用 library 校验，拒绝加载 Homebrew FreeRDP dylib（`mapping process and mapped file (non-platform) have different Team IDs`），App dyld 直接 `Abort trap: 6` 崩溃——正式签名反而起不来的坑。
  4. 重建 → 替换 /Applications → 重启；首次连 LAN 弹「本地网络」授权 → 允许 → 永久记住（TCC 按 Team ID + bundle id 记）。
- **预防复发（维护纪律）**：
  - Apple Development 证书一年有效（本次 2027-09-08 到期）。到期后在 Xcode 重新生成证书，并把 `tauri.conf.json` 的 `signingIdentity` SHA-1 同步更新（或改用证书 CN）；`security find-identity -v -p codesigning` 是 0 valid 时先查 WWDR 链是否过期（KI 本条目第 2 条）。
  - `signingIdentity` 是**机器特定**的：推给 GitHub Actions 构建 release 前必须处理（CI 无此证书会构建失败；CI 需 `APPLE_SIGNING_IDENTITY`/证书 secrets 或临时移除该段回退 ad-hoc）。
  - 只要 App 还运行时链接 Homebrew FreeRDP（`libfreerdp-client3.3.dylib`），`hardenedRuntime` 必须保持 `false`。
  - **三问速判**：终端 ssh 同 IP 通？App 连 Tailscale(100.x) 通？`codesign -dv` 无 Team ID？三连全中 = 本条目（本地网络 TCC + 无签名身份），别再当网络/路由问题修。
- **状态**：✅ 已修复并真机验证（2026-09-08，192.168.2.20 直连桌面正常）。

## KI-040 — 多会话剪贴板失效：后台会话 CLIPRDR 从不接线/宣告（已随 0.3.9 发布，真机回归待用户）

- **症状（用户报告）**：同时开两个远程桌面时，只有一个支持复制粘贴，另一个完全不通。
- **根因（两层）**：
  1. `jr_clip_monitor_ready` 只在 `s->clip_text` 非空时宣告 ClientFormatList，而初始快照仅由 `jr_clipboard_sync_start`（**只对聚焦会话调用**）存储 → 后台启动的会话永远不宣告，xrdp 认为该会话没有剪贴板通道，粘贴永远无效。
  2. 宣告的懒重试由 0.5s 焦点轮询 timer 触发（KI-019 路径），后台会话无 timer → 无任何重试接线途径；切 Tab 后 timer 只在 pasteboard 再次变化时才入队，刚切换即粘贴必失败。
- **修复（2026-09-08）**：
  - `bridge.c jr_clip_monitor_ready`：无条件 `jr_clip_announce`（接线+宣告与焦点解耦，每会话 worker 自行完成握手）。
  - `macos_view.m jr_clipboard_sync_start`：快照存储后额外 `jr_session_enqueue_local_clipboard_text` 入队一次，Tab 切换即宣告，无需等 pasteboard 变化。
  - 产品语义保持不变（KI-022）：剪贴板跟焦点会话；远端→Mac 仅焦点可写（generation guard 照旧）。
- **状态**：✅ 已随 0.3.9 发布（2026-09-08）；真机双桌面剪贴板回归 PENDING（验收：`grep '\[jr-clip\]' ~/Library/Logs/jetson-remote.log` 两个 session 地址都有 format-list sent；两桌面切换后粘贴均生效）。

## KI-041 — 卡死桌面关闭后重连永久转圈：shutdown join 无界阻塞整个启动链（已随 0.3.9 发布，真机回归待用户）

- **症状（用户报告）**：运行一段时间后一个桌面卡死（画面不动）；关掉该 Tab 后重新连接，永远停留在「连接中」转圈，再也连不上。
- **根因**：卡死的 FreeRDP worker 线程不退出（可挂在 `freerdp_client_start`，xrdp 半死时握手无响应）；`RdpSession::shutdown` 的 `worker.join()` **无界阻塞**；`launch_session` 命令开头 `is_running_keyed → close_keyed → shutdown()` 卡死 → Tauri IPC 永不返回；前端 `relaunch` 的 `await gateway.launch` 永不 resolve，而 KI-036 的 8 次重连链只在 launch **reject** 时才触发→ tab 停在 launching 转圈。
- **修复（2026-09-08）**：
  - `session.rs shutdown`：`join_with_deadline`（4s，50ms 轮询）；超时放弃回收（泄漏 worker+C ctx，日志 `[jr-flow] rdp worker join timeout`），保证 close/launch 有界。
  - `bridge.c jr_session_disconnect`：置位后续发 `SetEvent(s->cmd_wake)` 立即唤醒事件循环。
  - `bridge.c jr_session_connect`：`FreeRDP_TcpConnectTimeout=10000`，冻结的 connect 阶段有上界。
  - 前端 `sessionsStore.relaunch`：`withHardTimeout` 120s 硬天花（合法最坏 ≈75s），超时按 `rdp_failed` 落 error 可点击重试，即使用户遇到新的后端卡死也不再「永久转圈」。
- **状态**：✅ 已随 0.3.9 发布（2026-09-08）；真机卡死场景回归 PENDING。设备侧卡死根因（KI-015 xfwm4 段错误嫌疑或他因）待真机日志另开子任务诊断。

## KI-042 — 远程桌面实时画面「一帧一帧浮现」：客户端零编码协商 + 连接类型非 LAN（已修复，真机验证）

- **症状（用户报告）**：把 App 当远程 GUI 显示器用——在 Jetson 上跑实时目标检测 / 摄像头预览（`cv2.imshow`），画面不流畅，**一帧一帧浮现**（slideshow / 逐步刷出），不是连续视频。
- **量化基线（Thor dk，1280×700 全屏噪声）**：远程产出端 **151.9 fps**，端到端交付仅 **1.7 fps**。产出端能生成的帧比通路能承载的多 ~90 倍 → **脚本不是瓶颈**。
- **根因（两个独立且都必须满足的条件，任一缺失都静默退化为 legacy bitmap 路径）**：

  1. **连接类型必须是 `CONNECTION_TYPE_LAN`(0x06)，不能用 AUTODETECT**。xrdp 只在客户端 Info PDU 里声明 LAN 时才创建编码器：
     `xrdp/xrdp_encoder.c:60` — `if (client_info->mcs_connection_type != CONNECTION_TYPE_LAN) return 0;`
     该字段由 `libxrdp/xrdp_sec.c:2126` 从客户端 Info PDU 读出。
     FreeRDP **库**默认值是 `AUTODETECT`(0x07)，`xfreerdp` 的默认也是——所以连 README 里那条手工验证用的 `sdl-freerdp` 命令**同样没有编码器**。
  2. **必须显式开启 bitmap codec**。FreeRDP **库 API** 把每个 codec 都留成 FALSE：`settings.c` 从不给 `RemoteFxCodec`/`NSCodec`/`JpegCodec` 赋值（calloc 零值），`SupportGraphicsPipeline` 由 `ServerMode` 推导（客户端恒 FALSE）。只有 CLI 会开（`cmdline.c::prepare_default_settings()` → `freerdp_set_connection_type()`），而**只有 AUTODETECT 分支会开**——LAN 分支不会。两处都没做到时 `rdp_write_bitmap_codecs_capability_set()` 发出去的 `bitmapCodecCount == 0`，xrdp 解析出 `rfx_codec_id = 0`。

  后果：客户端与 `sdl-freerdp` 都在 legacy bitmap/order 路径上跑画面（1280×724×4B ≈ 3.7 MB/帧，摄影类内容压缩比接近 1:1）。
- **真机证实（修复前）**：`~/.xorgxrdp.<display>.log` 只有 `jpeg support 0`，**没有** `got RFX capture` / `got H264 capture`——而 `libxorgxrdp.so` 里只有这两条 capture 模式日志串，对应 `xorgxrdp/module/rdpClientCon.c:887-905` 仅有的两个加速分支（`capture_code==2` RFX / `==3` H264）。
- **修复**（`native/freerdp_bridge/bridge.c` `jr_session_connect`，在 `freerdp_client_start` 之前）：
  ```c
  freerdp_set_connection_type(settings, CONNECTION_TYPE_LAN);   /* 必须 LAN */
  freerdp_settings_set_bool(settings, FreeRDP_RemoteFxCodec, TRUE);
  freerdp_settings_set_bool(settings, FreeRDP_JpegCodec, TRUE);
  /* + 6 个桌面装饰性能提示压到省流量侧 */
  ```
- **修复后真机结果（Thor dk，同设备同内容 A/B）**：

  | | 交付 fps | emit 速率 |
  |---|---|---|
  | 修复前（legacy bitmap） | **1.7** | 33.6/s |
  | 仅开 codec + AUTODETECT（无效中间态） | — | — |
  | codec + LAN（RFX 生效） | **6.3** | **1032/s** |
  | codec + LAN + 呈现节流 | **6.3** | **12.5/s** |
  | 同上，真实内容（渐变+移动方块） | **14.5** | 16.5/s |

  设备端确认：`rdpClientConProcessMsgClientInfo: got RFX capture` + `cap_width 1280 cap_height 768`。J501（xrdp 0.9.17 / xorgxrdp 0.2.17）同样成立。
- **伴生修复（N-03 / RDN-02，同一处代码）**：原事件循环在**每条解码 PDU 批次后**都发射一帧整帧，RFX 生效后变成 ~1000 次/秒的 `malloc`+`memcpy`+`dispatch`（≈3.8 GB/s），反过来饿死同一个 worker 线程上的解码。改为 `jr_maybe_emit`：按 33ms 限速 + `EndPaint` 置脏才发射，并保留「GFX 已连 / 500ms 心跳」安全网。emit 1032/s → 12.5/s（82×），**但不改善帧率**——证明发送洪水不是帧率瓶颈。
- **剩余上限（设备侧，不在本项目可控范围）**：运行期间 Xorg 进程占单核 **54–90%**，系统 86–90% 空闲 → 瓶颈是 xorgxrdp 单线程、**无 SIMD** 的 RFX 编码（`module/rdpSimd.c` 的 SIMD 为 x86 专属，aarch64 全走标量 C）。理论上限 25 fps（`MIN_MS_BETWEEN_FRAMES 40`，0.9.x 无配置开关）未触及，即**编码跟不上而非节流**。要突破需 xrdp ≥ 0.10.2 + `--enable-x264` + `gfx.toml`（Ubuntu 包不提供，需源码构建）。
- **可观测性（防复发）**：新增 `[jr-nego]` 单行日志，每次连接打印：
  `[jr-nego] connType=6 rfx=1 rfxId=0 jpeg=1 jpegId=0 ns=0 nsId=0 gfx=0 gfxDvc=0 mfrs=3948544`
  期望形状：`connType=6 rfx=1 gfxDvc=0`（xrdp 0.9 无 EGFX，故 GFX 恒 0）。
  > `rfxId=0` 是**正常**的：xrdp 在自己的 Demand Active 里把 RFX 的 codecID 写成 0（"client sets"），真正的 id 由客户端在 Confirm Active 里给出。
- **服务端 oracle**：`ssh <jetson> 'grep -E "got RFX capture" ~/.xorgxrdp.10.log'`。客户端 `[jr-nego] rfx=1` 但服务端无此行为 → 失败在 xrdp 侧（先查 `mcs_connection_type` 与 `xrdp_caps_process_codecs`）。
- **验收门禁**：App 的交付帧率必须 **≥** 同一隧道下 `sdl-freerdp` 的对照值。注意默认参数的 `sdl-freerdp` 走 AUTODETECT，本身也没有编码器；要拿它当「好」的对照需 `sdl-freerdp /network:lan /rfx:on ...`。
- **状态**：✅ 真机验证通过（2026-09-15，Thor dk + Robotics-J501 双设备）。1.7 → 6.3 fps（最坏情形）/ 14.5 fps（真实内容）。

## KI-043 — 闪退 + 卡死：会话生命周期上的 `JRView` UAF 与主线程死锁（已修复，真机验证）

用户报告的「用一半闪退卡死」是**两个独立的 P1**，都落在会话生命周期这条路径上。它们不是本次视频修复引入的：崩溃的是 `/Applications` 里的 **0.3.9 release 二进制**（11:14 启动、13:33 崩溃），不含本轮改动。两条都已被项目自己的两轮审计记录（`docs/audit/2026-09-14`、`2026-09-15`）为 `still-open`，且**从未修复**。

现场崩溃栈（决定性证据）：

```
Thread 26 Crashed:  EXC_BAD_ACCESS (SIGSEGV)
0 libobjc.A.dylib   objc_retain + 16
1 jetson-remote     jr_view_present_buffer + 52
2 jetson-remote     on_frame + 508
3 jetson-remote     jr_session_connect + 340     ← FreeRDP worker 事件循环
```

### A. `JRView` use-after-free（= 审计 P1-5 / RD-02 / RDN-05）→ 闪退

- **机制**：`RdpSession::shutdown` 的有界 join（4s）**超时**分支（`session.rs` 的 `None` 分支）只把 `session`/`context` 置空，C session、`Box::into_raw` 的 `SessionContext` 与 worker **全部刻意泄漏且仍在运行**；但 `_view: NativeView` 仍是结构体字段 → 结构体 drop → `jr_view_destroy` → 异步块 `__bridge_transfer` + `removeFromSuperview` → **JRView 被 dealloc**。被放弃的 worker 从阻塞点恢复后仍会发射（`bridge.c` 的循环条件只在迭代顶部判定），`on_frame` 于是拿悬垂指针调 `jr_view_present_buffer`；该函数的逃逸 block 捕获 `__strong v`，`build.rs` 用 `-fobjc-arc` 编译 → block **copy 时** `objc_storeStrong` → 在 **worker 线程**上 `objc_retain(已释放)`。
  > 审计第一轮曾写「block 不 retain」，第二轮已更正；**不得沿用旧表述**。
- **三个入口**，任一都会触发：
  1. 上面的 4s join 超时分支；
  2. `RdpSessionManager` 无 `Drop` + 退出只关隧道 → managed state 析构时每个 `RdpSession` **不经 `shutdown`** 直接 drop；
  3. **同 key 并发 `launch_session`** → `sessions.insert` 静默顶替并 drop 旧 session。`Background` 会话的视图从未挂载、没有 superview 持有，**结构体一 drop 立刻 dealloc，下一个循环 tick（≤33ms）就崩** —— 这条最像用户遇到的那种「用着用着突然没了」。
- **修复**：
  - **视图所有权转移给回调上下文**：`SessionContext.view` 改为持有 native view 的 `Arc`（`FrameTarget` 特征对象），`RdpSession.view` 也持一份。`Box::from_raw(context)` 只在 **join 成功**分支执行，所有泄漏路径都保留 Box → worker 能回调期间视图**不可能**被释放。正常路径引用计数 2→1→0，销毁时机与修复前完全一致。配套给 `NativeView` 补 `unsafe impl Sync`（`Arc<T>: Send` 要求 `T: Send + Sync`）。
  - **`impl Drop for RdpSession` + `detach_fast()`**：覆盖入口 2/3。无阻塞、不碰主队列同步点（`Drop` 可能在管理器锁被持有时运行，`attach_input` 是 `dispatch_sync(main)`）；`jr_clipboard_sync_stop` 由 `clipboard_active` 守卫（否则后台会话析构会静默杀掉焦点会话的剪贴板同步，KI-040 回归）。worker 未结束时**刻意泄漏视图**，与 C session 同生命周期。
  - **C 侧发射栅栏**：`disconnecting` 改为 `LONG` + 互锁；`jr_maybe_emit` 入口短路。被放弃的 worker 不再向已泄漏的视图持续整帧拷贝。**明确不做**：在 `jr_session_disconnect` 里把 `cb.on_frame_updated` 置空 —— 那两个相邻非原子字段被 worker 分两次读，ARM64 上会读出「旧函数指针 + 新 NULL user」而制造新崩溃。

### B. 主线程永久死锁（= 审计 P1-11 / RC-05 / RDN-01）→ 卡死

- **机制**：`RdpSessionManager::focus()`（同族还有 `launch_entry`/`hide_all`/`clear_exited`）**同时持有 `sessions` 与 `focused` 两把锁**，跨越 `refocus()` → `attach_input` → `jr_view_attach_input`，而后者是 `macos_view.m` 里的 **`dispatch_sync(main)`**。主线程同时跑**同步** Tauri 命令 `rdp_status`/`session_status`/`all_session_statuses`，它们要取同一把 `sessions` 锁 —— 而 `all_session_statuses` 被前端**每 1000ms** 轮询。互等 → 主线程永久冻结，无超时，只能强杀。
- **修复**：
  - 三个命令改为 `#[tauri::command(async)]`（非 async 函数加 `(async)` 走 tokio 线程，签名/返回类型/前端契约全不变）。主线程退出等待者角色后，环无法形成。
  - `clear_exited` 把阻塞的 `shutdown()` 移出锁（原来在锁内跑 4s join + `dispatch_sync`）。
  - `run()` 记录主线程 `ThreadId`，`focus`/`launch_entry`/`hide_all` 入口加 `debug_assert_ne!` —— 把「未来有人加一个同步命令就重新引入永久冻结」变成构建期失败。
  - 同 key 并发启动加 RAII in-flight 声明，重复时返回既有 `RdpLaunchResult::AlreadyRunning`（同时消除入口 3）。

### 验证（headless，无需 ASan）

新增 `src-tauri/src/bin/session_lifecycle_probe.rs`：在进程主线程构造真实 `RdpSession`（无 Tauri 窗口），走完整 teardown，并用 `jr_view_live_count()` 断言视图生命周期。配套注入器 `JR_RDP_STALL_AFTER_DISCONNECT_MS` 让 4s 超时分支**确定性可复现**（该分支平时罕见）。

| 场景 | 修复前 | 修复后 |
|---|---|---|
| abandon 路径（注入 8s 卡顿） | 视图被释放（`views after drop = 0`），复现器判定 FAIL / 进程崩溃 | `[jr-flow] rdp worker join timeout` + `views = 1`，**exit 0** |
| 正常路径（无注入） | 视图随结构体释放（正确） | `views after drop = 0`，exit 0（干净回收） |

**阴性对照**：把上下文的视图持有退回裸指针（= 修复前的真实结构）后，同一复现器立刻判定 `views after drop = 0` 并 **exit 4** —— 证明该复现器确实能抓到本 bug，不是自证。配套单测覆盖 `may_reclaim` 门禁与并发启动拒绝。

### 仍有意的泄漏（与设计一致，非遗漏）

abandon 路径（4s 超时、退出、并发顶替）会刻意泄漏：worker 线程 + C `jr_session_t` + `SessionContext` + 它的 native view。理由与 KI-041 一致 —— 保 `shutdown` 有界返回优先于回收。视图此时已 `remove_from_window()` 离屏，因此不显示、不挡 UI。

### 未修的同类项（另立任务）

- **N-04**：剪贴板全局态（`g_clipSession` 等）+ 0.5s 定时器的第二个 UAF 窗口（`jr_clipboard_sync_stop` 无 session 参数）。
- **N-15**：`s->gdi` 数据竞争（主线程鼠标事件的 `jr_session_get_size` vs worker 的 `gdi_free`）。
- 审计第 6 条建议里的结构性锁重构（transition lock + UI 状态外移），本轮以命令异步化 + 断言约束替代。

- **状态**：✅ 已修复并 headless 真机验证（2026-09-15，J501 `100.93.59.109`）。崩溃/死锁的**用户现场**复现仍建议在 0.3.10 上跟一轮。

## KI-044 — 远程桌面里改 WiFi 报 "Not authorized to control networking"：polkit 不认 xrdp 远程会话（已修复，自愈）

- **症状（2026-09-22 真机）**：在 App 的远程桌面里打开网络设置改 WiFi，弹 `Connection failure / Connection activation failed / (1) Not authorized to control networking.` 桌面本身完全正常，只有 NM 相关的操作被拒。
- **根因（三层叠加）**：
  1. Ubuntu 自带的 NM polkit 规则只对 `subject.local && subject.active && (sudo || netdev)` 放行；**xrdp 会话是 remote 会话，`subject.local == false`** → 规则从不匹配。
  2. 于是回落到 NM policy 默认值：`enable-disable-network` 的 `allow_inactive=no` **直接拒绝**（就是那句报错的来源）；`settings.modify.system` 的 `allow_inactive=auth_admin_keep` 需要交互认证。
  3. 会话里**没有任何 polkit agent**（未装 `policykit-1-gnome`/`xfce-polkit`，autostart 只有 xfce4-screensaver），所以连弹窗授权都走不到 → 一律失败。
- **诊断要点**：设备 polkit `0.105-33`，`/usr/lib/polkit-1/` 下只有 `polkit-agent-helper-1`、`libpolkit` 无 mozjs 依赖 → **没有 JS 后端，`/usr/share/polkit-1/rules.d/*.rules` 在这里是死文件**，必须写 `.pkla`。真机 `pkcheck` 探针（远程会话下）：`enable-disable-network` → **rc=1**（未授权，实锤），`network-control` / `settings.modify.system` → rc=2。用户 `seeed` 在 `sudo` 组、不在 `netdev` 组。
- **修复（自愈）**：
  - `bootstrap.sh` 新增幂等步骤 `fix_nm_polkit`：写 `/etc/polkit-1/localauthority/50-local.d/47-nm-remote-session.pkla`（`50-local.d` 优先级高于 `10-vendor.d`，可覆盖 vendor 那几条只给 `ResultActive=yes` 的规则）。**三个 Result 全给**：`ResultAny`（SSH 等无会话）、`ResultInactive`（remote 会话，本次失败的分支）、`ResultActive`（本地桌面）。
  - 身份写 `unix-user:$SESSION_USER;unix-group:netdev`：只按组授权不够——`adduser` 之后**当前进程的组列表不会刷新**，本次会话的 `pkcheck` 仍判失败，设备会永远进不了 ready；按用户授权立刻生效，组身份留给其它/后续用户。同时把用户加入 `netdev` 与 Ubuntu 惯例对齐。
  - `check-environment.sh` 新增 `nm_polkit_ok` 事实（`pkcheck --process $$`）；Rust `classify` 将其纳入 `service_ok` → 缺失即 Broken → 走既有自愈装机。
- **坑（真机才暴露）**：幂等判定的 `[ -f "$PKLA_FILE" ]` 必须走 `as_root` —— `/etc/polkit-1/localauthority/` 是 `drwx------ root`，非 root 穿不过父目录，`[ -f ]` 恒假，会让每次运行都白白重写一遍。
- **状态**：⚠️ 设备端修复已落地并真机验证（2026-09-23，`192.168.2.20`）：`nm_polkit_ok` 由 `false` → `true`、幂等复跑通过。**会话内那条分支（`ResultInactive`）仍需在 xrdp 桌面里跑一次 `pkcheck` 确认**，SSH 里验的是 `ResultAny` 分支，验不到本次失败的那条。
## KI-045 — 剪贴板双向全死：sesman `ListenAddress` 让 chansrv 改走 TCP，xrdp 永远握不上手（已修复，自愈）

- **症状（2026-09-22 真机）**：远程桌面显示、键鼠、声音全部正常，唯独**剪贴板双向全死** —— Mac 复制的内容粘不进远端，远端复制的也带不出来。桌面毫无异常，剪贴板静默失效。
- **根因（配置，一条因果链）**：xrdp 0.9.17 `sesman/chansrv/chansrv_config.c` 里 chansrv 的 `use_unix_socket` **只**由这一条推导，且默认 0：
  ```c
  #define DEFAULT_USE_UNIX_SOCKET 0
  if (g_strcasecmp(name, "ListenAddress") == 0)
      if (g_strcasecmp(value, "127.0.0.1") == 0)
          cfg->use_unix_socket = 1;
  ```
  为 0 时 `setup_listen()` 走 `else` 分支，去监听 **TCP `7200 + display`**（本机 :10 → 7210）；而 xrdp 前端只会连 `/run/xrdp/sockdir/xrdp_chansrv_socket_<display>`（`strings /usr/sbin/xrdp` 里只有这个路径）。**两边永远对不上** →
  `xrdp_mm_connect_chansrv: connect failed trying again...`（4 次 × 4s）→ `error in trans_connect chan` → 该连接此后再不重试通道服务 → chansrv 虽然活着，CLIPRDR/rdpdr/rdpsnd 全废。
  本机 `sesman.ini` 被改成 `ListenAddress=0.0.0.0`（`dpkg -V xrdp` 正标记 `sesman.ini`/`xrdp.ini` 与包原始 MD5 不一致），诱因很可能是早先为「让 xrdp 监听所有网卡」而改（见 KI-006）。
- **真机证据链**：chansrv 横幅 `UseUnixSocket (derived): false` 且 `ss -ltnp` 显示 `xrdp-chansrv` 监听 `*:7210`；改回 `127.0.0.1` 后横幅立刻变 `true`（同一条推导），`/var/log/xrdp.log` 从 `connect failed` 变为 **`xrdp_mm_connect_chansrv: chansrv connect successful`**，客户端日志随之首次出现 `server capabilities received` + `monitor-ready`。
- **客户端侧证据（排除客户端）**：修复前每次连接都是 `jr_clip_wire` 成功 → `sending client capabilities` 发出 → 服务端 `ServerCapabilities`/`MonitorReady` **永不到达** → C 侧 `clip_ready` 恒为 0 → `jr_clip_announce` 直接 return。客户端这段逻辑本身正确。
- **附带修掉的客户端 bug**：`macos_view.m` 的 `g_suppressNext` 是**会被 latch 住的一次性标志** —— `jr_mac_clip_set` 在同一主线程块里已把 `g_lastCount` 同步更新，于是下一 tick 在 `c == g_lastCount` 处提前 return、标志不被消费；此后 Mac 侧的**第一次真实复制会被静默吞掉**（恰好一次）。回声抑制本就是结构性的（`g_lastCount`/`g_lastText` 同步更新即足够），该标志纯属冗余，已移除。
- **修复（自愈）**：
  - `bootstrap.sh` 新增幂等步骤 `fix_sesman_listen`：把 `/etc/xrdp/sesman.ini` 的 `ListenAddress` 写成 `127.0.0.1`（键缺失时插入 `[Globals]`，因为**缺失同样使推导为 false**）。它同时把 `XRDP_RESTART_NEEDED` 置位，强制 `enable_service` 重启服务 —— 服务「已经 active」不代表跑的是新配置，chansrv 只在启动时读一次 sesman.ini。
  - `check-environment.sh` 新增**静态可判**事实 `sesman_listen_local`；`classify` 将其纳入 `service_ok` → 命中即 Broken → 自动走 bootstrap 修好。**刻意不查运行期 chansrv 状态**：进程与监听 socket 只在活跃会话期间存在，而且 chansrv 接受连接后会主动删掉自己的监听 socket，查那个文件完全不可靠。
  - `clear_stale_chansrv()`：任何服务重启前先 `pkill -x xrdp-chansrv` + 清残 socket。`xrdp.service` 带 `RuntimeDirectory=xrdp`，重启会整个重建 `/run/xrdp`，把存活 chansrv 的 socket 抹掉而它自身不会重建 —— 残留会让下一次登录继续失败。
  - 客户端：C bridge 新增 `jr_session_clip_ready()`（暴露既有 `clip_ready`）→ `clip_ready_keyed()` → `launch_session` 在桌面可用后给 CLIPRDR 3s 宽限（健康时 MonitorReady 在连接握手期就到，**正常路径零额外延迟**），未就绪则落进既有修复路径重连一次，仅告警不阻断桌面。
- **验证（2026-09-23 真机 `192.168.2.20`）**：先人为把 `ListenAddress` 改回 `0.0.0.0` 制造故障态（`check-environment.sh` 正确报 `sesman_listen_local: false`），再重启 App —— **App 全自动完成 探测→classify→bootstrap 改写→重启→重连**，`ListenAddress` 被自动改回 `127.0.0.1`，设备端出现 `chansrv connect successful`，客户端出现 `monitor-ready`。
- **仍未闭环**：剪贴板**数据面**的最终确认。用 `python3-gi`(GTK) 在会话内直接读写 X 剪贴板做无头验证时，观察到服务端仍未 `FormatDataRequest`、chansrv 也未接管 X selection 所有权 —— 即通道虽已协商，数据流可能仍未打通。**需要一次真人复制/粘贴确认**（这是唯一可靠的判据）。若确认仍不通，下一步是取 chansrv 的 DEBUG 日志（`EnableSyslog=true` 但 `SyslogLevel` 默认 INFO，DEBUG 被挡；需临时把 `[ChansrvLogging] SyslogLevel=DEBUG`）再定位。
- **状态**：⚠️ 通道层已修复并真机验证；数据面待真人确认。
