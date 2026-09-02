# PRD — Jetson Remote

> 本文档只写产品需求与验收标准，不含技术设计（见 ARCHITECTURE.md）与执行清单（见 IMPLEMENTATION_PLAN.md）。

## 1. 产品定位

面向 NVIDIA Jetson 的**零配置、无显示器（headless）远程桌面客户端**。

一句话卖点：

```
输入 IP + 用户名 + 密码 → Connect → 桌面
```

用户无需理解 XRDP / Xorg / xorgxrdp / XFCE / FreeRDP / SSH Tunnel / systemd / Wayland —— 全部藏在产品内部。

## 2. 背景与问题

Jetson 常被装在机器人 / AMR / AGV / 边缘 AI 盒子 / reComputer / 服务器 / 实验与户外设备里，无显示器、无键鼠。用户想从 Mac / PC 访问图形桌面。

现有方案（VNC / NoMachine / XRDP 手配 / RustDesk / HDMI Dummy Plug / Fake EDID）普遍存在：无 HDMI 黑屏、需改配置文件、Wayland/GNOME/Xorg 冲突、要懂 display session、安装复杂、JetPack 升级失效、体验不统一。

本项目解决：**用户只需知道 IP、用户名、密码，即可进入 Jetson 图形桌面。**

## 3. 用户与场景

- **目标用户**：Jetson 开发者 / 边缘部署工程师 / Seeed 生态用户。
- **核心场景**：
  1. 首次连接：从干净 Jetson（无 HDMI）一键完成配置并进入桌面。
  2. 再次连接：免重装，直接进桌面。
  3. 断线重连：恢复**原有 session**（不丢工作区）。

## 4. MVP 产品原则

1. **零配置**：收敛到 `IP / 用户 / 密码 → Connect`，不输出教程。
2. **Headless First**：优先"无 HDMI 也能稳定建远程桌面"。物理桌面 `:0` 与远程桌面 `:10` 是**两个 session**（设计选择，非 bug）。
3. **稳定 > 画质**：第一阶段只保证 Terminal / 编辑器 / Browser / 文件管理器 / Python GUI / 网络设置 / Docker / Jupyter / 普通 Linux GUI。
4. **MVP > 架构完美**：禁止自研 RDP / 视频协议 / WebRTC / FreeRDP FFI / compositor / daemon。

## 5. 功能需求（MVP）

### 5.1 连接
- 输入 Host(IP)、用户名、密码，点 Connect。
- 提供可选的「Remember this device」：V0.3 起勾选后连接成功即持久化设备（host/username）与密码（macOS Keychain），响应启动自动重连；不勾选连接已记住设备即遗忘。

### 5.2 自动化管线（首次连接）
SSH 登录 → 识别 Jetson → 识别 Ubuntu/JetPack → 检测远程桌面组件 → （缺失则）自动安装并配置 → 启动服务 → 验证 → 建立安全连接 → 打开桌面。

### 5.3 设备识别
动态识别（**不写死 SKU**），至少涵盖 `uname -m`、`/etc/os-release`、`/etc/nv_tegra_release`、`dpkg-query nvidia-l4t-core`，返回 is_jetson / architecture / l4t / jetpack / device_model 等。

### 5.4 会话保持
断开后**不销毁** XFCE/Xorg session；再次连接恢复到已有 session（`DISPLAY :10` 保留）。

### 5.5 UI（严格 3 个核心状态）
1. **Home**：连接表单（IP/用户名/密码/记住设备/Connect）。
2. **Provisioning**：阶段进度（SSH→检测→安装→配置→启动→验证，默认隐藏命令输出，可「Show details」）。
3. **Error**：错误转译为人话（SSH 超时 / 认证失败 / 非 Jetson / sudo 失败 / XRDP 失败）。

### 5.6 诊断
可复制的 `DiagnosticsReport`（Host OS / App 版本 / Jetson 信息 / 组件版本 / 服务状态 / 端口 / 脱敏日志），**绝不含密码**。

## 6. 非功能需求

### 6.1 安全（P0 红线）
- SSH **host key 校验**：首次连接显示指纹，TOFU 记录；指纹变化给安全警告，不默认 accept-all。
- **密码不落盘**（默认仅内存）、不进日志/tracing/argv/URL/analytics。
- sudo 密码经管道 stdin 传输，**禁止** `echo PASSWORD | sudo` 出现在 history/argv/日志。
- FreeRDP 密码**禁止**作为 command-line argument（用 `/from-stdin`）。
- 任意日志等级都需 **secret redaction**。

### 6.2 幂等
bootstrap 重复运行不破坏系统：已装→跳过、配置已对→跳过、服务已 active→跳过。

### 6.3 可靠性
App 重启 / Jetson 重启 / RDP 断线 / Wi-Fi 短暂断开 / XRDP 重启 / 重复 bootstrap / 错密码 / 错 IP / SSH 超时 / 非 Jetson 设备 —— 均不能 crash / 卡死 / 无限 loading。

### 6.4 状态\[机\]
用单一 `ConnectionState` 枚举（Idle → ConnectingSsh → Authenticating → DetectingDevice → … → Connected → Error），**禁止 boolean 堆**。

## 7. 支持范围

| 维度 | P0 | P1 |
|---|---|---|
| Host 客户端 | macOS (Apple Silicon) | Windows x86_64 |
| Jetson 系统 | JetPack 7.x / Ubuntu 24.04 | JetPack 6.x / Ubuntu 22.04 |
| 设备 | Orin Nano / NX / AGX Orin / Thor（动态识别） | |

> MVP 第一阶段：客户端仅 macOS AS，但架构保证可跨平台扩展；不因 Windows 阻塞 macOS。

## 8. 使用前提（Jetson 侧）

仅需：已启动、连局域网、知道 IP、SSH Server 可达、有 sudo 用户。**无需** HDMI / 显示器 / 键鼠 / 预装 XRDP / 预装 XFCE。

## 9. 验收标准

### 9.1 核心 DoD（唯一成功标准）
一台**无 HDMI 的干净 Jetson**，仅输入 IP/用户名/密码即可自动：SSH 登录→识别→检测→装 XRDP→装/配 XFCE→起服务→验证→安全连接→起 FreeRDP→看到 XFCE 桌面，且可操作、可断开重连恢复 session。

### 9.2 功能验收
可移动鼠标 / 点击 / 键盘输入 / 打开 Terminal / 文件管理器 / Browser / 复制文本 / 调窗口尺寸 / 断开 / 重连 / 恢复 session；至少验证 1920×1080。

### 9.3 可靠性验收
见 §6.3 场景全覆盖。

### 9.4 性能目标
鼠标无严重拖尾、终端输入接近实时、窗口拖动基本流畅、滚动可接受。本地记录 SSH 连接 / 配置 / RDP 启动 / 重连耗时指标（**不上传 telemetry**）。

## 10. 非目标（进 ROADMAP，MVP 不做）

mDNS 发现、SFTP 文件管理、集成终端、摄像头、jtop/GPU 监控、电源模式、Docker 管理、服务管理、USB 重定向、音频/麦克风、多屏、4K、WebRTC、VirtualGL、TurboVNC、游戏串流、FreeRDP FFI、嵌入式 RDP Canvas、云中继、账户、云后端、telemetry 平台。