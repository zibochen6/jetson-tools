# ROADMAP — Jetson Remote

> MVP 之后才考虑的功能。MVP 期间一律不开发。

## V0.2 — 发现
- mDNS Auto Discovery（研究 `mdns-sd` / Avahi）
- Nearby Jetsons 列表（`● Online`）
- **FreeRDP 4B 打包**：自包含 arm64 `.app`（tauri-plugin-shell sidecar + otool/install_name_tool dylib，或 static-lean 自构建）——消除「需 brew install freerdp」与 Homebrew 运行时依赖（见 DECISIONS ADR-029）

## V0.3 — 便利
- Device History
- Credential Keychain（macOS Keychain / Windows Credential Manager，经 `CredentialStore`）
- Auto Reconnect
- Clipboard 改进

## V0.4 — 集成工具
- Integrated Terminal
- SFTP File Browser

## V0.5 — 系统面板
- Jetson System Panel：CPU / GPU / RAM / 温度 / 电源模式 / JetPack / CUDA

## V0.6 — Agent
- `jetson-remote-agent`：mDNS / metrics / power / services / camera

## V1 — 嵌入式桌面
- FreeRDP Library → Native rendering surface → Embedded Desktop（主应用内页面，非独立窗口）

## V2 — 高性能模式
- GPU/DRM Capture → GStreamer → H264/H265 → WebRTC（对应 `WebRtcTransport`）

## 明确不做的（当前）
音频 / 麦克风 / 多屏 / 4K / USB 重定向 / 游戏串流 / TurboVNC / VirtualGL / 云中继 / 账户 / 云后端 / telemetry 平台。