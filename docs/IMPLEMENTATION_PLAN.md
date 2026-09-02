# IMPLEMENTATION_PLAN — Jetson Remote

> 状态标记：`[ ]` Todo · `[~]` 进行中 · `[x]` 完成 · `[!]` 阻塞
> 单个 Task 目标粒度 30–120 分钟。需求 PRD.md / 架构 ARCHITECTURE.md / 决策 DECISIONS.md。

## Phase 0 — Technical Spike ✅

- [x] P0.1 `scripts/remote/detect.sh`（结构化 JSON 检测）
- [x] P0.2 `scripts/remote/bootstrap.sh`（幂等装机，含 `.xsessionrc` 自愈）
- [x] P0.3 FreeRDP 连接验证（`/from-stdin` 凭据 + SDL）
- [x] P0.4 断线重连恢复 session 验证
- [x] P0.5 `docs/SPIKE_RESULTS.md`（踩坑记录）
- 验收：真机 `seeed@192.168.100.164` 全链路通过（见 SPIKE_RESULTS）

## Phase 1 — Tauri 骨架 ✅

- [x] T1.1 初始化 Tauri 2 + React 19 + TS + Vite 7 + Tailwind 4（`pnpm create tauri-app` react-ts）
- [x] T1.2 Home 表单页（IP/用户名/密码 show-hide/记住设备/Connect，校验驱动 disabled）
- [x] T1.3 Provisioning 进度 stepper（5 步产品语言，按 `ConnectionState` 派生）
- [x] T1.3b Ready 屏（设备卡 + "Opening Desktop…"）
- [x] T1.4 Error 页（`describeError` 语义映射 + Retry/Back）
- [x] T1.5 `ConnectionState` / `ConnectionProgress` / `JetsonDevice`（无 password）/ 错误码类型
- [x] T1.6 `connectionStore`（Zustand 无 persist）+ `ConnectionService` 抽象 + `MockConnectionService` + DevScenarioPicker
- [x] T1.7 Dev 占位屏（desktopMocked，不伪造 connected）
- [x] T1.8 Rust 命令边界 `commands/app.rs`（app_info + health_check）+ 窗口 620×720
- 验收：`cargo tauri dev` 出窗口；`pnpm typecheck/lint/test(23)/build` 全绿；`cargo fmt/clippy/test` 全绿

## Phase 2 — SSH 层 ✅（代码 + 单测完成；真机端到端待设备恢复）

- [x] T2.1 `ssh::client`：russh 0.63.1 连接生命周期（password auth，ring 后端）
- [x] T2.2 `ssh::handler` + `ssh::types`：host key TOFU（`tofu_decision` 纯函数 + fingerprint）
- [x] T2.3 `ssh::executor`：`RemoteExecutor` trait + `StreamCollector`（含 1MB 输出上限）
- [x] T2.4 `device::detector` + `types`：`include_str!` 内嵌 detect.sh，`sh -s` stdin 执行
- [x] T2.5 `commands::connection::probe_device`：高层单命令（ephemeral session + typed ProbeResult/ProbeError）
- [x] T2.6 `trust.rs`：hosts.json 信任存储（app_config_dir，非敏感）
- [x] T2.7 前端：`TauriConnectionService` + `ConnectOutcome` + `HostKeyPromptScreen` + store trust/replace/取消不回弹
- [x] T2.8 单测：Rust 11（TOFU / channel 聚合 / detect fixture / malformed / not-jetson / 密码脱敏）+ 前端 33
- [!] 真机端到端：`192.168.100.164` 已断电/离线（ping 100% 丢包、`No route to host`），7 项真机测试待设备恢复后补测
- 验收：`pnpm/cargo` 全检查绿；真机验证见 docs/TESTING_JETSON.md（待补）

## Phase 3 — 自动 Provisioning

## Phase 3 — 自动 Provisioning ✅

- [x] T3.1 `scripts/remote/check-environment.sh`（只读 JSON 事实）+ `bootstrap::checker` `include_str!` 内嵌
- [x] T3.2 `check()`：读环境事实 + `classify()` 纯函数 → `Ready/Partial/Broken/ProvisionRequired`
- [x] T3.3 `bootstrap::provisioner`：sudo preflight(`-S -p '' -v`) → `mktemp` 上传 → `sudo bash` 流式解析 `[bootstrap] step=X` → cleanup
- [x] T3.4 `bootstrap::verifier`：bootstrap 后重查环境，exit0≠Ready，必须真实复验
- [x] T3.5 `RemoteExecutor::exec_with_stdin_lines`：真·流式按行回调（`SshSession` 用 `provision_timeout`，Mock 默认行切分）
- [x] T3.6 `prepare_remote_desktop` command + `ipc::Channel<ProvisionEvent>` 流式进度 + `PrepareResult`
- [x] T3.7 前端：`ConnectionService.prepare` + store prepare 流/`provisioningLocked`/`environment` + 新错误码 `sudo_required`/`verification_failed`
- [x] T3.8 单测：Rust 20（classify/stage 解析/sudo 映射/秘密脱敏/provision 流）+ 前端 35
- 真机：Ready 快路径（不 provision）✅ + 安全 repair（stop xrdp→Broken→provision→Ready→幂等重跑不 provision）✅
- 验收：`192.168.100.164` 一键 check/provision/verify 闭环通过（见 docs/TESTING_JETSON.md）

## Phase 4 — FreeRDP Sidecar ✅（4A 功能集成完成；4B 打包留待下一轮）

- [x] T4.1 `rdp/` 模块：`types`/`args`/`error`/`process`/`freerdp`/`client`(trait `RdpClient`)/`manager`（7 文件）
- [x] T4.2 `rdp::freerdp` `FreeRdpSidecarClient`：定位 `sdl-freerdp`（`RDP_BINARY_PATH` env → 已知路径 → PATH）+ `--version` preflight
- [x] T4.3 `rdp::process`：`tokio::process` spawn → `RdpProcess`（stdin 写 `/args-from:stdin` 凭据 → close stdin → 300 行 ring buffer drain → SIGTERM/宽限/SIGKILL close → Drop 杀孤儿）
- [x] T4.4 单测：**FreeRDP argv 不含密码**（`/args-from:stdin` 一参一行）、`/cert:tofu` 非 ignore、进程状态机（`/bin/sleep` 真子进程）、already-running、error 映射、Debug 红密
- [x] T4.5 UI：`connect → ready → auto-launch → desktop_opened`；`DesktopRunningScreen`（Close/Disconnect + 1s 轮询退出）；`ReadyScreen` Open Desktop；错误码 `rdp_client_missing`
- 验收：真机 `/args-from:stdin` + `/p:<creds>` + `/cert:tofu` → `reconnected session display :10.0`（sesman 日志确证 XFCE session 复用）；cargo 40 / pnpm 44 全绿；`cargo tauri dev` 启动成功
- [ ] 4B（下一轮）：tauri-plugin-shell sidecar + 自包含 arm64 .app（见 DECISIONS ADR-029 / ROADMAP）

## Phase 5 — SSH 隧道

- [ ] T5.1 `ssh::tunnel` `LocalPortForward`（direct-tcpip），动态端口
- [ ] T5.2 FreeRDP 改为连 `localhost:<port>` → Jetson `127.0.0.1:3389`
- [ ] T5.3 Jetson 侧 XRDP 绑 localhost（`xrdp.ini` 改造，幂等）
- 验收：不开 3389 到 LAN，RDP 经隧道可用

## Phase 6 — 可靠性

- [ ] T6.1 reconnect/retry/timeout；进程清理；SSH 重连；RDP 崩溃检测
- [ ] T6.2 `DiagnosticsReport`（脱敏：Host OS/App 版本/Jetson 信息/组件版本/服务状态/端口/sanitized 日志）
- [ ] T6.3 可靠性用例（错密码/错 IP/超时/重复 bootstrap/XRDP 停服）→ Error 语义
- 验收：9 项真机清单全部通过；`Copy Diagnostics` 无密码

## Phase 7 — Polish

- [ ] T7.1 UI 细节 / 文案 / 深浅主题
- [ ] T7.2 图标 / 窗口尺寸
- [ ] T7.3 设备历史（不含密码）
- 验收：外观与交互打磨完成

---

## 风险/依赖（同步 DECISIONS/KNOWN_ISSUES）
- russh `Handler` 样板过重 → 降级 ssh2（T2.1 gate）
- FreeRDP `/from-stdin` 版本差异 → 已真机验证 3.31 OK
- macOS 本地网络权限 → Phase 4/6 实测
- JetPack 7.x 未真机验证 → 动态识别（detect.sh 已覆盖 6.x/7.x 映射）