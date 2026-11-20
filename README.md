# Jetson Remote

> Remote desktop for NVIDIA Jetson, without a monitor.

```
1. Enter your Jetson IP
2. Enter username and password
3. Connect

No HDMI. No VNC setup. No display configuration.
```

## 这是什么

零配置的 Jetson 无显示器（headless）远程桌面客户端。输入 IP + 用户名 + 密码，应用自动完成 SSH 登录 → 识别 Jetson → 检测/安装远程桌面 → 建立安全连接 → 打开 XFCE 桌面，支持断线重连恢复会话。0.3.0 起支持**多设备同时连接**；0.3.2 修复后台重连抢占与隧道凭据隔离；**0.3.3 P0 稳定化（单 owner 命令队列、剪贴板 generation、中文输入法、隧道错误细分、JetPack 5.x 支持）；0.3.4 修复新设备（Ubuntu 24.04）首次连接必败（KI-033）；0.3.5 修复输入法组合期非文本键绕过 IME 导致的「无法打字/组合框卡死」（KI-034：组合期间一切键走输入法、命令选择器映射回 scancode、keyUp 对称）。 0.3.6 修复关闭设备标签后桌面不消失（KI-035：原生视图从不卸载、最后一帧永久盖住首页；teardown 先让出屏幕 + 关闭最后一个标签显式回到总览）。**0.3.7 修复设备重启后反复「连不上」与重连体验**（KI-036：半开隧道被误判健康，改为端到端健康检查 + 失败即废弃重拉，前端统一有界重连链与启动窗口重试；KI-038：sysctl 禁用 IPv6 抹掉 lo 的 ::1 导致 xrdp 永远连不上 sesman，环境检查新增检测并自动修复；XRDP 修复时清理陈旧会话）。0.3.8 修复 macOS 本地网络 TCC 拦截（KI-039：正式签名 + application-identifier entitlement）。**0.3.9 双会话剪贴板与卡死重连修复**（KI-040：CLIPRDR 接线/宣告与焦点解耦，两个桌面的剪贴板各自可用；KI-041：卡死会话的 worker join 改为 4s 上界，关闭/重连不再永久转圈）+ v0.4 Reliability 工程底座（可靠测试门禁、设备信任矩阵、更新器事务引擎、Provisioning V2 协议、嵌入式会话接口冻结层）。

（XRDP / xorgxrdp / XFCE / FreeRDP / SSH Tunnel 都是实现细节，用户无需理解。）

## Jetson Remote 和 VNC 怎么选

**用 Mac 调试几台没接显示器的 Jetson，选 Jetson Remote 可以省去逐台配置远程桌面的工作。要和现场操作员一起看检测窗口、复现报错，选 VNC 的屏幕共享模式。** VNC 也能创建无显示器的虚拟桌面。下面以 TigerVNC 为例，区分这两种用法。

| 比较项 | Jetson Remote | VNC（TigerVNC） |
| --- | --- | --- |
| 首次连接 | SSH 可达时，输入 IP 和账号密码，应用会检查环境，按需安装 XRDP 和 XFCE，并建立 SSH 隧道。首次装包需要 `sudo` 权限。 | 镜像已预装并配置好 VNC 时，打开客户端即可连接。从零部署时，需要选择桌面模式并配置服务。 |
| 无显示器开发 | 默认创建独立的 XFCE 桌面，无需 HDMI。断开后会话保留，重连可继续工作。 | 虚拟桌面模式（Xvnc）同样无需 HDMI，适合已统一部署 VNC 的设备。 |
| 现场协作 | 远程开发者可以独立开终端、调程序。现场屏幕上已经弹出的报错不会出现在这个桌面。 | 屏幕共享模式让双方看到同一个窗口，适合现场排障。远程鼠标和键盘也会直接操作现场桌面。 |
| 客户端与多设备 | 面向 macOS，可以保存多台 Jetson，在一个应用里用标签切换。 | 有 Windows、Linux 和 macOS 客户端，方便团队跨平台使用。 |
| 连接与维护 | 应用管理 SSH 隧道，并能检查和修复部分 XRDP 配置问题。连接依赖设备上的 SSH 和远程桌面服务。 | 可直连配置好的服务，也支持 TLS 或 SSH 隧道，便于沿用团队已有的运维配置。 |
| Jetson 端内存 | 独立的 Xorg 和 XFCE 会话占用内存，断开但保留会话时，这部分内存仍在使用。 | 共享已有桌面通常更省新增内存，因为只需提供屏幕共享。若用 Xvnc 另开 XFCE，也要计入一套桌面的开销。 |

### 性能与资源占用

在 Thor 开发套件上，Jetson Remote 使用 RFX 编码传输约 720p 的测试画面。客户端实际收到的帧率随内容变化：渐变背景加移动方块为 **14.5 fps**，全屏随机噪声为 **6.3 fps**。测试期间，Jetson 端 Xorg 进程占用**单个 CPU 核心的 54%–90%**，瓶颈出在设备端的画面编码。[KI-042 真机测试记录](docs/KNOWN_ISSUES.md)

VNC 的编码、画质和画面变化范围也会影响 CPU 和带宽。目前项目缺少同一台 Jetson、相同画质和负载下的 VNC 对照，无法给出两者在帧率、内存、带宽或操作延迟上的排名。对相机或 YOLO 应用，应分别看 **Jetson 上的推理帧率**和**客户端收到的预览帧率**。上面的数字只反映合成画面的远程显示性能。[TigerVNC 编码设置](https://tigervnc.org/doc/vncviewer.html)

部署与会话行为可查阅[项目真机验证记录](docs/TESTING_JETSON.md)、[TigerVNC 屏幕共享说明](https://tigervnc.org/doc/x0vncserver.html)和[虚拟桌面说明](https://tigervnc.org/doc/Xvnc.html)。

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

> **画面流畅度对照**：上面这条默认命令走 `AUTODETECT` **不启用编码器**，而已嵌入引擎会。要拿它当「好」的基准测帧率，必须显式带上 `/network:lan /rfx:on`——xrdp 只在客户端声明连接类型为 LAN 时才创建 RFX/JPEG 编码器。细节与实测见 `docs/KNOWN_ISSUES.md` KI-042 与 `docs/EMBEDDED_RDP.md` §7.1。

```bash
# 流畅度基准（含编码器）
printf '<pass>\n' | sdl-freerdp /v:<jetson> /u:<user> /from-stdin /cert:ignore /network:lan /rfx:on
```

详见 `docs/TESTING_JETSON.md`。

### 开发指南（防止改坏连接链路）

- **改任何连接/输入/剪贴板代码前必读**：`docs/CONNECTION_REGRESSION_GUIDE.md`（链路全景、编码硬规则、回归清单、调试速查）
## 发布与自动更新

- **发布流程**：改完功能 → 同步 bump 四处版本（`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/tauri.conf.json`、`package.json`；preflight 还会校验 tag）→ 打 tag 推 GitHub（`v0.3.x`）→ Actions 自动构建并发布 Release（macOS dmg + `Jetson Remote.app.tar.gz`，Linux amd64 `.deb`）。
  - 本地手动发布：`scripts/release.sh <version>`（走 gh CLI）。
- **自动更新**：稳定版 app 底栏有「检查更新」按钮 → 拉到最新 Release 后「下载并安装」→ 自动替换当前 app 并重启。
  - 开发版（`tauri dev`）只提示新版本并给下载页链接，不做自替换。
  - 更新只支持文本应用替换：仅在本机 `/Applications` 或 `~/Applications` 中**已安装 app 形态**运行时生效。
- **隧道**：app 用系统 `/usr/bin/ssh` 自动建立环回隧道（SSH/RDP 双平面走 127.0.0.1，规避 macOS 本地网络隐私限制 KI-004，见 KI-021），无需手动配置；密码仅经 0700 目录里的 `SSH_ASKPASS` 脚本传递。
- **签名（macOS 15+/27 必读）**：本地网络授权要求带 Team ID 的正式签名，ad-hoc 构建的 LAN 腿会被 TCC 静默拒绝（「No route to host」，KI-039）。本机 `tauri.conf.json` 已配置 `bundle.macOS`（signingIdentity/entitlements/hardenedRuntime）——**该 signingIdentity 是机器特定证书**：换机、证书到期（一年）需更新其 SHA-1；推 GitHub Actions 发版前需处理 CI 签名（否则缺少该证书会构建失败，见 KI-039 维护纪律）。

## Linux 支持（Ubuntu 24.04 amd64）

- **安装**：从 [Releases](https://github.com/zibochen6/jetson-tools/releases) 下载 `Jetson.Remote_<version>_amd64.deb`（本地 `tauri build` 产物的文件名是 `Jetson Remote_<version>_amd64.deb`，上传 GitHub 后空格会变成点），然后 `sudo apt install ./Jetson.Remote_*.deb`。
- **系统要求**：Ubuntu 24.04+ **amd64 桌面**（glibc 2.39、`libgtk-3-0t64`、WebKitGTK 4.1）。22.04/20.04 与 arm64 不在支持范围。
- **桌面嵌入**：GTK3 内嵌视图，远程桌面从窗口顶部 44px 标签栏下方开始；鼠标/键盘、Ctrl 组合键、中文输入法（IBus/Fcitx，preedit 不发送、提交走 Unicode）与双向文本剪贴板按 macOS 同样的契约实现。真实 24.04 桌面上的 GUI 交互验收仍在进行，CI 目前覆盖安装与启动阶段。
- **自动更新**：Linux 版**不做**自替换安装；「检查更新」只显示 Release 页面链接，下载新 `.deb` 后由 `apt` 安装。
- **本地构建**：

  ```bash
  sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev \
    freerdp3-dev libcjson-dev libfuse3-dev \
    libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libgl1 libegl1
  pnpm install
  pnpm tauri build --bundles deb
  ```

- **CI**：`.github/workflows/linux-ci.yml` 在 `ubuntu-24.04` 上跑前端/Rust 测试、构建 `.deb`、在干净的 `ubuntu:24.04` 容器里 `apt install ./deb` + `ldd` + Xvfb 启动冒烟。
