# Jetson Remote

> Remote desktop for NVIDIA Jetson, without a monitor.

```
1. Enter your Jetson IP
2. Enter username and password
3. Connect

No HDMI. No VNC setup. No display configuration.
```

## 这是什么

零配置的 Jetson 无显示器（headless）远程桌面客户端。输入 IP + 用户名 + 密码，应用自动完成 SSH 登录 → 识别 Jetson → 检测/安装远程桌面 → 建立安全连接 → 打开 XFCE 桌面，支持断线重连恢复会话。0.3.0 起支持**多设备同时连接**；0.3.2 修复后台重连抢占与隧道凭据隔离；**0.3.3 P0 稳定化**：输入/剪贴板全部改为单 owner 命令队列（FreeRDP API 只由 worker 线程调用，修复「剪贴板修好鼠标坏」的线程根因）、剪贴板所有权带 generation 防跨会话串台、支持 macOS 中文输入法（NSTextInputClient + RDP unicode 输入）、「连不上」细分为 6 类隧道错误、多设备隧道决策加 trace 与单测、bootstrap 支持 JetPack 5.x（Ubuntu 20.04）设备。

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

# 真机联调：app 会自动用系统 ssh 建环回隧道（KI-021），直接启动即可
cargo tauri dev
# 可选（仅 dev）：复用手动开的隧道，跳过自建隧道
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

### 开发指南（防止改坏连接链路）

- **改任何连接/输入/剪贴板代码前必读**：`docs/CONNECTION_REGRESSION_GUIDE.md`（链路全景、编码硬规则、回归清单、调试速查）
## 发布与自动更新

- **发布流程**：改完功能 → 同步 bump 三处版本（`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`package.json`）→ 打 tag 推 GitHub（`v0.3.x`）→ Actions 自动构建并发布 Release（dmg + `Jetson Remote.app.tar.gz`）。
  - 本地手动发布：`scripts/release.sh <version>`（走 gh CLI）。
- **自动更新**：稳定版 app 底栏有「检查更新」按钮 → 拉到最新 Release 后「下载并安装」→ 自动替换当前 app 并重启。
  - 开发版（`tauri dev`）只提示新版本并给下载页链接，不做自替换。
  - 更新只支持文本应用替换：仅在本机 `/Applications` 或 `~/Applications` 中**已安装 app 形态**运行时生效。
- **隧道**：app 用系统 `/usr/bin/ssh` 自动建立环回隧道（SSH/RDP 双平面走 127.0.0.1，规避 macOS 本地网络隐私限制 KI-004，见 KI-021），无需手动配置；密码仅经 0700 目录里的 `SSH_ASKPASS` 脚本传递。
