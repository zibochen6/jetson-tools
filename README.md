# Jetson Remote

> Remote desktop for NVIDIA Jetson, without a monitor.

```
1. Enter your Jetson IP
2. Enter username and password
3. Connect

No HDMI. No VNC setup. No display configuration.
```

## 这是什么

零配置的 Jetson 无显示器（headless）远程桌面客户端。输入 IP + 用户名 + 密码，应用自动完成 SSH 登录 → 识别 Jetson → 检测/安装远程桌面 → 建立安全连接 → 打开 XFCE 桌面，支持断线重连恢复会话。

（XRDP / xorgxrdp / XFCE / FreeRDP / SSH Tunnel 都是实现细节，用户无需理解。）

## 状态

- **Phase 0 Technical Spike ✅** —— 真机（reComputer J501 mini, JetPack 6.2.1）全链路已打通。见 `docs/SPIKE_RESULTS.md`。
- 目前处于 Phase 1（Tauri 骨架）待开工。

## 目录

```
docs/          PRD / 架构 / 实施计划 / 决策 / 已知问题 / 测试清单 / Roadmap / Spike 结果
scripts/remote/  detect.sh（设备检测）· bootstrap.sh（幂等装机）
src/           前端（React+TS，待建）
src-tauri/     Rust 后端（待建）
```

## 开发

```bash
# 依赖
brew install freerdp          # sdl-freerdp（RDP 客户端）
cargo install tauri-cli --locked

# 运行
cargo tauri dev

# 真机联调（未签名二进制被 macOS 本地网络权限静默阻断，见 docs/KNOWN_ISSUES.md KI-004）
# 终端 1：系统 ssh 回环隧道
ssh -L 2222:localhost:22 -L 3389:localhost:3389 <user>@<jetson>
# 终端 2：dev tunnel 模式启动，SSH/RDP 自动路由到 127.0.0.1
VITE_JR_SSH_PORT=2222 cargo tauri dev

# 检查
cargo fmt --check && cargo clippy && cargo test
pnpm typecheck && pnpm lint && pnpm test && pnpm build
```

## 真机验证（Phase 0 命令）

```bash
# 检测
ssh <user>@<jetson> 'bash -s' < scripts/remote/detect.sh
# 装机（sudo 密码经 stdin）
printf '<pass>\n' | ssh <user>@<jetson> 'bash /tmp/bootstrap.sh'
# 连接桌面（原生窗口）
printf '<pass>\n' | sdl-freerdp /v:<jetson> /u:<user> /from-stdin /cert:ignore
```

详见 `docs/TESTING_JETSON.md`。