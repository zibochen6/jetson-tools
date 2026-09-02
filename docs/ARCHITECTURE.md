# ARCHITECTURE — Jetson Remote

> 技术设计与模块划分。需求见 PRD.md，决策记录见 DECISIONS.md，执行见 IMPLEMENTATION_PLAN.md。

## 1. 顶层分层

```
               Jetson Remote (Tauri 2 + React + Rust)
                        │
              Product Experience Layer
                        │
        ┌───────────────┴───────────────┐
        │                               │
  Control Plane                    Desktop Plane
     (SSH)                            (RDP)
        │                               │
        └───────────────┬───────────────┘
                        │
                     Jetson
                        │
          XRDP → xorgxrdp → Xorg(:10) → XFCE
```

- **Control Plane（SSH）**：认证、设备检测、bootstrap、诊断、SSH 隧道 —— 控制"怎么配置、怎么连"。
- **Desktop Plane（RDP）**：FreeRDP sidecar —— 承载"看什么画面"。
- 两者解耦：未来 Desktop Plane 可换成 WebRTC / VirtualGL 而无需推翻产品（见 DECISIONS ADR 抽象边界）。

## 2. 客户端技术栈

| 层 | 选型 |
|---|---|
| 壳 | Tauri 2 |
| UI | React + TypeScript + Vite + Tailwind CSS（少量 Radix 或自研） |
| 状态 | Zustand（`connectionStore`）；项目小时允许先用 React state |
| 后端 | Rust：tokio / serde(_json) / thiserror / anyhow / tracing |
| SSH | `russh`（备选 `ssh2`） |
| RDP | FreeRDP 3.x sidecar（`sdl2-freerdp`/`sdl3-freerdp`），独立原生窗口 |

## 3. 目录结构

```
jetson-remote/
├── src/                       # 前端
│   ├── app/
│   ├── components/
│   ├── features/{connection,provisioning,devices,settings}/
│   ├── stores/                # connectionStore 等
│   └── lib/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/          # Tauri command 入口（前端唯一调用点）
│   │   ├── ssh/{client,auth,executor,tunnel}/
│   │   ├── device/{detector,parser,model}/
│   │   ├── bootstrap/{installer,verifier,scripts}/
│   │   ├── rdp/{client,freerdp,process}/
│   │   ├── tunnel/
│   │   ├── credentials/       # CredentialStore（MVP 仅内存）
│   │   ├── diagnostics/
│   │   └── platform/
│   ├── binaries/              # sidecar binary 打包处
│   └── Cargo.toml
├── scripts/remote/{detect.sh,bootstrap.sh}
├── docs/
└── README.md
```

## 4. Rust 后端模块职责

- **ssh/client**：一条 SSH 会话的生命周期管理。
- **ssh/auth**：password + publickey 认证；host key 校验与 finger\_print。
- **ssh/executor**：实现 `RemoteExecutor`（执行命令、流式返回 stdout/stderr + exit code）。
- **ssh/tunnel**：`LocalPortForward`（direct-tcpip）—— Mac `localhost:<动态端口>` → Jetson `127.0.0.1:3389`。
- **device/detector**：编排 `detect.sh` 并调用 parser；**device/parser**：纯函数解析 L4T / os-release / dpkg 样本（重点单测）。
- **bootstrap/installer+verifier**：`check_remote_environment` / `provision_remote` / `verify_remote_environment`，消费 `scripts/remote/*.sh`。
- **rdp/client**：`RdpClient` trait；**rdp/freerdp**：`FreeRdpSidecarClient`；**rdp/process**：进程 spawn→stdin 写凭据→跟踪退出。
- **diagnostics**：`DiagnosticsReport`（脱敏）。

## 5. 关键抽象（仅这些提前抽象；未来必替换）

| Trait | 当前实现 | 未来 |
|---|---|---|
| `RemoteExecutor` | `RealSshExecutor` | `MockRemoteExecutor`（测试） |
| `DesktopTransport` | `RdpTransport` | `WebRtcTransport` / `VirtualGlTransport` |
| `RdpClient` | `FreeRdpSidecarClient` | `FreeRdpEmbeddedClient` / `WebRtcClient` |
| `CredentialStore` | 内存（MVP 不持久化） | macOS Keychain / Windows Credential Manager |

规则（来自 PRD §49 Rule3）：只有**两个实际实现**或**明显系统边界**才抽象；以上四处满足"明显边界"。

## 6. 状态机

```
Idle → ConnectingSsh → Authenticating → DetectingDevice
  → UnsupportedDevice ┐
  → CheckingRemoteEnvironment → ProvisionRequired → Provisioning
  → Verifying → Ready → CreatingTunnel → LaunchingRdp → Connected
  → Disconnected / Error（任意点可回 Idle）
```

前端只据 `ConnectionState` 渲染，不做多 boolean 判断。

## 7. 数据模型

```rust
struct Device {
    id: String,
    host: String,
    port: u16,
    username: String,
    hostname: String,
    model: String,
    jetpack_version: String,
    l4t_version: String,
    ubuntu_version: String,
    architecture: String,
    remote_status: RemoteStatus,
    last_connected_at: Option<i64>,
}
```

**Credential 与 Device 分离**：`Device` 无 `password` 字段；凭据走独立的 `ConnectionCredentials`（仅内存）。

## 8. 网络拓扑（RDP over SSH tunnel）

```
FreeRDP ──▶ localhost:33990 ──(SSH local forward / direct-tcpip)──▶ Jetson 127.0.0.1:3389 ──▶ XRDP
```

- 只需 Jetson `:22`；不开防火墙 3389；降低攻击面；统一认证入口。
- 开发/验证期允许临时 LAN 直连 `:3389`；**正式 MVP 切回隧道**。
- 最终目标：Jetson XRDP 只绑 `127.0.0.1`。

## 9. 安全与凭据

- host key：TOFU，指纹存本地，变化即告警。
- 密码仅内存；FreeRDP 密码经 `/from-stdin`（stdin），禁 argv。
- 日志用 `tracing`，默认 INFO，任何等级禁记 password / private key / credential。
- sudo 密码经 SSH channel stdin 管道，免受 history/argv/进程列表暴露。

## 10. 技术决策索引

见 DECISIONS.md：XRDP(ADR-001)、XFCE(ADR-002)、FreeRDP sidecar(ADR-003)、SSH 隧道(ADR-004)、russh(ADR-005)、密码不落盘(ADR-006)、idempotent SSH 无 agent(ADR-007)。