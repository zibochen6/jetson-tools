# DECISIONS — Jetson Remote

> 轻量 ADR 记录。每个决策：标题 / 理由 / 后果。

## ADR-001 — 用 XRDP 而不是 VNC

- **决策**：Desktop Transport 采用 XRDP。
- **理由**：无需 HDMI / 显示器即可创建 headless 虚拟 session（xorgxrdp→Xorg）。
- **后果**：物理桌面 `:0` 与远程 `:10` 是两个 session（设计选择）。FreeRDP 客户端生态成熟（macOS/Windows 均有 SDL 客户端）。

## ADR-002 — 用 XFCE 而不是 GNOME

- **决策**：远程会话跑 XFCE。
- **理由**：避开 GNOME/Wayland compositor 冲突与登录黑屏。轻量、稳定、适合无 GPU 加速的远程场景。
- **后果**：MVP 不保证 GNOME 桌面体验；用户接受"远程桌面 = 轻量 XFCE"。

## ADR-003 — FreeRDP sidecar，不做 FFI 嵌入

- **决策**：MVP 把 FreeRDP 作为独立原生窗口（sidecar），不搞 FFI / framebuffer 嵌入 WebView。
- **理由**：先验证完整 remote workflow；FFI 嵌入风险高、投入大。
- **后果**：桌面暂时是独立窗口（MVP 可见差异）；抽象 `RdpClient` 便于未来换 `FreeRdpEmbeddedClient`。

## ADR-004 — RDP over SSH tunnel

- **决策**：优先经 SSH 隧道连 RDP，不把 3389 暴露在 LAN。
- **理由**：只需 `:22`；免开防火墙；降攻击面；统一认证入口。
- **后果**：复杂度 +1（隧道层）；开发期允许临时 LAN 直连，正式版切回隧道。

## ADR-005 — SSH 用 russh（备选 ssh2）

- **决策**：优先 `russh`（Rust 原生、Tokio、password+pubkey、direct-tcpip 本地转发）。
- **理由**：PRD 推荐；已核实支持 direct-tcpip 本地端口转发。
- **后果**：`client::Handler` 样板较重；若阻塞，降级 `ssh2`（libssh2 封装，API 更直接）。禁用 sshpass/expect 作为产品依赖。

## ADR-006 — 密码 MVP 不落盘

- **决策**：密码仅内存，不持久化；FreeRDP 密码经 `/from-stdin`；禁 argv。
- **理由**：安全红线；避免 ps / 进程列表 / crash report / 日志泄露。
- **后果**：「Remember device」只记设备不记密码；Keychain/Credential Manager 留到 V0.3。

## ADR-007 — idempotent SSH bootstrap，无自研 agent daemon

- **决策**：用 SSH + 幂等脚本（`scripts/remote/*.sh`）完成配置，不写 `jetson-remote-agent` 常驻服务。
- **理由**：MVP 无需长期驻留；幂等脚本可重复安全执行。
- **后果**：未来 mDNS/监控/遥测需引入 agent（ROADMAP V0.6）。

## ADR-008 — 跟踪机制用 docs/，不建 Trellis task

- **决策**：本仓库的规划/执行状态用 `docs/IMPLEMENTATION_PLAN.md` 等文档维护，不另建 Trellis task。
- **理由**：PRD 已显式指定 docs 结构，且要求减少来回询问。
- **后果**：`.trellis/` 脚手架保留但不作为本项目主跟踪路径。

## ADR-009 — 优先 Ubuntu distro 包，不 source build

- **决策**：XRDP/xorgxrdp/XFCE 优先 apt distro 包；记录版本并做兼容校验；包过旧/有问题再考虑 managed 安装。
- **理由**：生产稳定优先；避免 `git clone master + 编译` 的漂移风险。
- **后果**：受 distro 包版本的已知 bug 影响（记录到 KNOWN_ISSUES.md）。

## ADR-010 — 包管理器用 pnpm

- **决策**：前端用 `pnpm`（单 `pnpm-lock.yaml`），不混 npm。
- **理由**：11.10.0 已装、快、corepack 在位。
- **后果**：脚本全部用 `pnpm run`；`cargo tauri dev` 的 beforeDevCommand 已走 pnpm。

## ADR-011 — Tailwind CSS v4（`@tailwindcss/vite` 插件）

- **决策**：用 Tailwind v4 + Vite 插件，无 `tailwind.config.js`，入口 `@import "tailwindcss"`。
- **理由**：v4 是当前主线、配置更简；不引 AntD/MUI/Bootstrap。
- **后果**：`dark:` 走 `prefers-color-scheme`（媒体策略）。

## ADR-012 — Zustand 不加 persist 中间件

- **决策**：连接表单（含密码）只驻内存 store，**不持久化**。
- **理由**：安全红线（密码不落盘）；「记住设备」Phase 1 仅 UI。
- **后果**：有测试锁死「无 persist 中间件」；Phase 6/7 再做非敏感设备持久化与 Keychain。

## ADR-013 — eslint 钉在 v9

- **决策**：`eslint@^9` + `typescript-eslint@8`，不追 eslint 10。
- **理由**：typescript-eslint 8.x 官方支持到 eslint 9；eslint 10 过新，求稳。
- **后果**：flat config（`eslint.config.js`）；`@eslint/js@^9` 与 eslint 主版本对齐。

## ADR-014 — SSH 控制面用 russh

- **决策**：SSH 客户端用 `russh`（0.63.1），不调 `ssh` 二进制、不用 sshpass/expect。
- **理由**：Rust 原生、Tokio、password 认证、direct-tcpip 可扩展；密码经内存 API 不落 argv。
- **后果**：`check_server_key` 需实现 TOFU（见 ADR-016）；API 在 0.63 有「AuthResult / PublicKeyOrCertificate / Channel.data(AsyncRead)」新形态。

## ADR-015 — crypto 后端选 ring

- **决策**：`russh` 用 `ring` 后端（`--no-default-features --features ring,rsa,flate2,serde`）。
- **理由**：构建/跨平台成本最低（无 cmake/cc）；`aws-lc-rs` 是默认且维护更活跃但需 cmake 编译 BoringSSL。
- **后果**：一行 feature 切换可回退 `aws-lc-rs`；若 ring 维护状态成为顾虑再切。

## ADR-016 — TOFU host key 验证

- **决策**：`check_server_key` 决策抽成纯函数 `tofu_decision(expected, seen)`；未知→记录+中止（返回 HostKeyUnknown），不匹配→HostKeyChanged（绝不静默覆盖）。
- **理由**：PRD §33 安全红线；指纹用 OpenSSH 格式 `SHA256:`。
- **后果**：host 证书不做（`Certificate` 变体标 `SHA256:unknown`，见 KI-008）。

## ADR-017 — detect.sh 编译期内嵌，经 stdin 执行

- **决策**：`include_str!("../../../scripts/remote/detect.sh")` 内嵌，`sh -s` + channel stdin 注入；不在远端建 `/tmp` 临时文件。
- **理由**：单一事实来源（复用 Phase 0 已验证脚本）；避免 shell quoting/命令长度问题。
- **后果**：改脚本后需重新 build 才生效。

## ADR-018 — Phase 2 用临时（ephemeral）SSH 会话

- **决策**：`probe_device` = connect → auth → detect → return → drop；不做全局 session manager。
- **理由**：本阶段只探测；Phase 3 再决定「重连 vs 保持会话」。
- **后果**：Phase 3 provision 需重新连接（或届时引入 session 池）。

## ADR-019 — 信任存储 hostskey 用 app_config_dir/hosts.json

- **决策**：TOFU 信任存 `app_config_dir()/hosts.json`（`{ "host:port": {algorithm, fingerprint} }`），非敏感可持久化。
- **理由**：与标准 `~/.ssh/known_hosts` 语义等价但置于 app 配置目录；绝不含密码。
- **后果**：多用户/多端口按 `host:port` 键区分。

## ADR-020 — bootstrap.sh 是 provisioning 唯一事实来源

- **决策**：Rust 只调用 Phase 0 已验证的 `scripts/remote/bootstrap.sh`（内嵌 `include_str!`），绝不重写 apt/sed/systemctl 逻辑。
- **理由**：避免两套 provisioning 逻辑漂移；复用已真机验证的幂等 + `.xsessionrc` 兼容修复。
- **后果**：改脚本需重新 build；脚本 marker（`[bootstrap] step=X`）是进度事件唯一来源。

## ADR-021 — 脚本经安全临时文件上传

- **决策**：`mktemp /tmp/jetson-remote-bootstrap-XXXXXX.sh` → `cat > <path>`（stdin）→ `chmod +x` → 运行 → 无论成败 `rm -f`。
- **理由**：避免 predictable-path 竞态；不引入 SFTP transport；避免 shell quoting/长度问题。
- **后果**：失败路径也保证清理（finally 语义）。

## ADR-022 — sudo 密码走 SSH channel stdin

- **决策**：`sudo -S -p '' ` 从 stdin 读密码，经 `exec_with_stdin`/`exec_with_stdin_lines` 的 channel stdin 传入。
- **理由**：密码不出现在 argv/日志/临时文件/`ps`。
- **后果**：preflight 用 `sudo -v` 区分「密码错」与「非 sudoers」；MVP 假设 SSH 密码 == sudo 密码（未来经 `SshCredential/ProvisionCredential` 边界解耦）。

## ADR-023 — bootstrap 后重新验证，不信任 exit 0

- **决策**：`verifier::verify` = 重新 `check()` 环境，`state != Ready` 即 `VerificationFailed`。
- **理由**：exit 0 不保证 Ready（PRD §28）。
- **后果**：bootstrap 成功但 verify 失败 → 明确错误（非静默 Ready）。

## ADR-024 — apt 期间不支持破坏性取消

- **决策**：preflight/upload 可取消；真正 bootstrap 运行后 `provisioningLocked=true`，Cancel 失效并显示「保持开机」。
- **理由**：中断 apt 可能致 dpkg 不一致（PRD §31）。
- **后果**：App 关闭中断 provisioning → 记录 Known Issue（KI-010）。

## ADR-025 — FreeRDP 凭据经 `/args-from:stdin`，绝不进 argv

- **决策**：argv 仅 `["sdl-freerdp", "/args-from:stdin"]`，全量参数（含 `/p:<password>`）逐行写 stdin，写后立即 close stdin。
- **理由**：FreeRDP 3.31 `--help` 明确 `/args-from:stdin`「cannot be combined with any other arg；one argument per line」；密码只经 stdin，`ps`/Activity Monitor/crash report 均不可见。
- **后果**：已在真机验证「以该形式成功 reconnect 到 XFCE session（:10.0）」。`/from-stdin` 作为已证备份路径（SPIKE F2），需降级时记录。

## ADR-026 — 单活动 RDP 会话 + 托管状态，App 退出 `Drop` 杀进程

- **决策**：`RdpProcessManager`（`Mutex<Option<RdpProcess>>`，`tauri::State`）托管唯一 sidecar；重复 launch 返回 `AlreadyRunning` 不二次 spawn；`Drop` 调 `start_kill()` 防孤儿 `sdl-freerdp`。
- **理由**：MVP 单设备单桌面（PRD §16）；不建 session 池。
- **后果**：强制单条会话；退出感知靠前端 1s 轮询 `rdp_status`（后续可换 Tauri event push）。

## ADR-027 — 优雅关闭优先（SIGTERM → 宽限 → SIGKILL）

- **决策**：`RdpProcess::close` 先 `libc::kill(pid, SIGTERM)` + 2s 宽限，仍在跑才 `start_kill`（SIGKILL）。
- **理由**：FreeRDP 对 SIGTERM 走干净 `ERRCONNECT_CONNECT_CANCELLED` teardown（真机确认）；远端 Xorg :10/XFCE 会话保持（PRD §24）。
- **后果**：加 `libc` 依赖（仅 kill(2)）；SIGKILL 仅为兜底。

## ADR-028 — 诚实状态 `desktop_opened`，不伪造 `connected`

- **决策**：`spawned ≠ authenticated`（PRD §18）；MVP 新增状态 `desktop_opened`（独立窗口已开），`connected/creating_tunnel/disconnected` 留待 Phase 5/深层集成。
- **理由**：FreeRDP 无稳定 machine-readable「连接成功」信号（默认日志 `license BB_ERROR_BLOB` 为 XRDP 非致命杂讯）。
- **后果**：UI 显示「Desktop open」，退出感知 = 进程退出（`rdp_status` → `exited`）。

## ADR-029 — 4B 打包策略（记录，不实施）

- **决策**：Phase 4A 用系统 `sdl-freerdp`；Phase 4B 优先「Homebrew 二进制 + otool/install_name_tool 复制 dylib」为第一候选，static-lean（`WITH_FFMPEG=OFF` 等自构建）为备选。
- **理由**：Homebrew FreeRDP 3.31 顶层依赖 `libfreerdp-client3.3/libfreerdp3.3/libwinpr3.3` + `libSDL3` + `libSDL3_ttf`，本体再链 OpenSSL + FFmpeg（`WITH_FFMPEG=ON`）——依赖树重。
- **后果**：4B 需引入 `tauri-plugin-shell` sidecar（仅授权 `sdl-freerdp`，不开 `allow-execute`）+ codesign/Gatekeeper；本轮记录入 ROADMAP。

## ADR-030 — 记住设备 = JSON + Keychain，自动重连 once-guard；wire 层密码 Option 化

- **决策**：V0.3 实现「记住设备」：非敏感标识（host/username）存 `app_config_dir/remembered.json`（原子写）；密码存 macOS Keychain（`keyring` v3 apple-native，service=`com.jetsonremote.app.remembered-device`，account=`username@host`）。同时只记一台设备（last wins）。启动时 `get_remembered_device` 探测 `hasPassword`，有密码即自动重连（前端 once-guard 防 StrictMode 双发），无密码仅预填表单。
- **触发**：`remember=true` 且连接成功（device probed）才保存；`remember=false` 连接同一台已记住设备成功 → 遗忘；首页提供显式 Forget 按钮。记忆 I/O 均 best-effort，失败不阻断连接。
- **密码流动**：wire 层 `SshConnectionInput.password` / `RdpConnectionRequest.password` 改 `Option<String>`；None = 后端从 Keychain 解析（`remember::resolve_password`），**已存密码永不回传前端**。新错误码 `SAVED_PASSWORD_MISSING` / `RDP_PASSWORD_MISSING` → 前端 `saved_password_missing`（提示回表单输密码）。
- **替代方案**：`security` CLI（密码会进 argv，违反 PRD §67，否决）；security-framework 手写（SecItem 生命周期/弃用边界易错）；zustand persist / 明文 localStorage（违反 ADR-012，否决）。
- **后果**：`keyring` 依赖仅 macOS target；未签名/dev 构建可能每次二进制变化弹 Keychain 访问授权（签名包无此问题，记入 KNOWN_ISSUES）。Rust 侧 `SshConnectionInput` Debug 恒 redact（None 也显示 `<redacted>`）。
## ADR-031 — 设备身份 v3：machine-id + 必填显名 + 多路径（LAN/Tailscale 自动选路）

- **决策**：一台设备 = 稳定 `deviceId` + 必填 `displayName` + 可变 `paths[{kind,address}]`。
  `deviceId` 优先取 `/proc/device-tree/serial-number`（Jetson 模组出厂序列号，每板唯一、重装不变），
  无序列号时回退 `/etc/machine-id`，两者皆无则回退 host 身份（legacy）。
  **真机发现**：两台测试 J501 为同一克隆镜像，`/etc/machine-id` 完全相同（hostname 也同为
  `seeed-desktop`），machine-id 单独作身份会把两块板并成一台 —— 序列号是必要修正。
  记忆库升 v3（`{"version":3,"devices":[…]}`，写永远 v3；v2/v1 只读透传为 deviceId=null 的 legacy 行）；
  密码 account 改 `user@deviceId`（legacy 行沿用 `user@host`）。会话 id、隧道 key、TOFU 身份
  统一 `username@deviceId`（缺 machine-id 时回退 host）。用户输入地址只是入口：连接前对所有候选
  （记忆 paths ∪ 本轮输入）并行 TCP 探测 `:22`（800ms），按 RTT 升序尝试，SSH 超时/认证失败换下一条；
  成功后用 detect.sh 上报的当前 IPv4 列表**覆盖** paths（过滤回环/docker 172.17/16/USB gadget
  192.168.55/24/链路本地；100.64/10 → tailscale，其余私网 → lan，公网丢弃）。
- **触发**：同一块板 LAN 与 Tailscale 两个 IP 曾被记成两台设备、开两个桌面、存两份密码。
- **显名**：新 machine-id 在探测成功后、provision/开桌面之前弹必填起名屏（不可跳过）；
  Tab/总览/已记住列表主标题只显示显名，当前路径做小字。已连设备再拿另一条 IP 进来 →
  复用 keyed session（focus 既有 Tab + 提示「已作为「某某」连接」），禁止一板两桌面。
- **合并**：连上后把同 username 且地址相交的 legacy v2 行合并进 machine-id 行：
  secret 复制到 `user@deviceId`（目标缺失时）→ 删除 legacy 行与旧 secret。
- **认证**：控制面只走密码；russh 传输错误中含 permission denied / authentication 字样的一律映射
  `AuthenticationFailed`（不向 UI 露出 PublicKey remaining-methods）；`resolve_password` 有 deviceId
  时精确取 `user@deviceId`，多设备时仍禁止用 127.0.0.1 猜密码。
- **替代方案**：hostname 作稳定 ID（两台板可能同为 `seeed-desktop`，否决）；SSH host key 指纹作
  身份（重装机即变、且 TOFU 前拿不到，否决）；网段扫描发现路径（越权、慢，否决——只记上报地址）。
- **后果**：remembered.json / secrets.json account 键变更（自动迁移，旧密码连上即合并）；
  hosts.json TOFU 键从 `host:22` 变为 `deviceId:22`（按指纹一次性自动迁移，不重复弹窗）；
  `probe_device_paths` 新 IPC；detect.sh 输出新增 `machine_id` / `ipv4_addresses`（旧后端字段兼容，
  serde default）。
