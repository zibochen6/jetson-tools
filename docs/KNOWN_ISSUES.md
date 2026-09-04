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
- **根因**：`gdi_init()` 只接线 **legacy GDI** 路径（BitmapUpdate/绘图指令/SurfaceBits），**不注册 RDPGFX 图形管线**（`gdi_StartFrame/EndFrame/SurfaceCommand/CreateSurface/UpdateSurfaces`）。XRDP 默认协商 RDPGFX → 桌面经 GFX 通道下发，未接 `gdi_graphics_pipeline_init(gdi, gfx)` 时**整帧被静默丢弃**，`primary_buffer` 全零 → 纯黑且 `EndPaint` 不触发。参考客户端在 RDPGFX 动态通道 connect 时懒调用该函数（`client/common/client.c:1610`）。
- **影响**：所有走 RDPGFX 的 XRDP 桌面；`endpaint=0` 是典型信号（「连上了却收不到帧」）。
- **修复**：`bridge.c` `jr_pre_connect` 里 `PubSub_SubscribeChannelConnected(instance->context->pubSub, freerdp_client_OnChannelConnectedEventHandler)`（默认处理器内部已含 RDPGFX→gdi 接线），post_disconnect 反订阅。另因 GFX 更新不保证调用 legacy `EndPaint`，RDP event loop 每 33ms 提交一次已解码的 `primary_buffer`，使 native view 不会停在占位黑色。见 `EMBEDDED_RDP.md` §7。
- **状态**：✅ 真机（SSH 隧道）视觉确认：XFCE 桌面与终端内容均可见（2026-09-02）。

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
