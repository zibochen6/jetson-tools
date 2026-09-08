# ChatGPT 优化方案·第二轮（无真机约束下的第一阶段执行包）

> 日期：2026-09-08 · 追问由 Jetson Remote 项目发出（响应第一轮 TOP1–4 拆解请求）
> 原始快照：docs/chatgpt-optimization-2026-09-08-r2-raw-snapshot.txt

下面我把 TOP1–4 收敛成一个**“无 Jetson 真机阶段的 Reliability Foundation 执行包”。目标不是模拟 Jetson 到 100%，而是先把所有与硬件无关、确定性强、最容易造成长期回归的部分**做到可自动验证。真机恢复后，再补最后一层 Compatibility / Soak / Power-cycle 验证即可。

路径我按你之前 Tauri 项目的习惯，以 apps/desktop/src-tauri/ 为基准；如果当前仓库目录略有不同，保持模块边界即可。

编辑
Jetson Remote v0.4 Reliability Release
无 Jetson 真机第一阶段执行包
0. 本阶段的 Definition of Done

这一阶段不要试图证明：

JetPack 5/6/7 真机全部稳定。

没有硬件做不到，也没必要伪装成做到了。

本阶段真正应该证明的是：

Connection orchestration 是确定的
Provisioning protocol 是确定的
RDP session lifecycle 是确定的
Updater recovery 是确定的
Device trust decision 是确定的

最终形成：

                  ┌────────────────────┐
                  │ Reliability Tests  │
                  └─────────┬──────────┘
                            │
            ┌───────────────┼────────────────┐
            │               │                │
            ▼               ▼                ▼
     Provisioning       Session Core      Updater Core
        Contract           Contract           Contract
            │               │                │
            ▼               ▼                ▼
      Ubuntu VM        Fake RDP Adapter    Sandbox App
            │               │                │
            └───────────────┼────────────────┘
                            │
                         macOS

等 Jetson 真机回来以后，只增加：

JetPack Compatibility Layer
+
Real FreeRDP / XRDP Soak
+
Power Cycle

而不是重新改架构。

1. 先建立一个共享原则

TOP1–4 不要分别自己造测试工具。

建议先增加：

apps/desktop/src-tauri/crates/
├── jr-core/
├── jr-test-support/
├── jr-provision/
├── jr-session/
├── jr-updater/
└── jr-device-trust/

不一定必须真的拆 6 个 crate。

如果现在代码规模不适合拆 crate，也至少保持：

src/
├── core/
├── test_support/
├── provision/
├── session/
├── updater/
└── trust/

但有一个模块很值得独立：

jr-test-support

里面统一提供：

FakeSshServer
FakeTunnel
FakeRdpAdapter
FakeClock
FakeKeychain
FakeFilesystem
FaultInjector
EventRecorder
ScenarioBuilder

以后所有可靠性测试都基于它。

不要每个模块自己 mock 一套。

2. TOP1 — Reliability Harness
2.1 无真机阶段能做到哪几层

建议现在建立 4 层。

真机回来后再加第 5 层。

Layer 0：Pure Core Tests

运行环境：

macOS
cargo test
pnpm test

不产生：

SSH
FreeRDP
XRDP
Linux VM

重点测试所有：

State Machine
Identity
Retry
Backoff
Port Allocation
Error Mapping
Redaction
Session Routing

这是最重要的一层。

应该非常快。

Layer 1：Deterministic Integration Harness

仍然只在 macOS。

但是引入：

Fake SSH
Fake tunnel
Fake RDP
Fake Keychain
Fake updater filesystem

测试整个连接 orchestration。

例如：

Connect
 ↓
SSH success
 ↓
detect success
 ↓
provision satisfied
 ↓
tunnel created
 ↓
RDP connected

这里不需要真实网络。

它测试的是：

Jetson Remote 自己有没有正确组织这些组件。

Layer 2：Linux Provisioning Contract

使用：

Ubuntu container
或
Ubuntu VM

建议：

Container：
测试 shell / JSONL / config mutation / parser

VM：
测试 systemd / xrdp / xorgxrdp / XFCE

因为完整 XRDP bootstrap 放 Docker 里做，很容易因为 systemd/PAM/DBus 环境差异产生假问题。

VM 更接近真实环境。

至少准备：

Ubuntu 20.04 ARM64/x86_64
Ubuntu 22.04
Ubuntu 24.04

CPU 架构不是这一阶段重点。

绝大多数 provisioning 问题来自：

apt
systemd
PAM
shell
config
permissions

这些都可以提前验证。

Layer 3：macOS Native Session Tests

测试：

RdpSessionManager
Native surface lifecycle
focus
resize
clipboard ownership
IME ownership
multi-session
cleanup

第一阶段可以：

FakeRdpAdapter

为主。

如果 Ubuntu VM 中跑得起 XRDP，则额外加入：

macOS FreeRDP
        ↓
Ubuntu VM XRDP

作为 integration smoke test。

Layer 4：Jetson Matrix

现在只定义接口。

暂时：

#[ignore]

或者：

requires_jetson

不执行。

未来恢复：

JP5
JP6
JP7

再启用。

2.2 第一批测试用例

以下建议直接作为 test function 名。

A. Connection State
connection_happy_path_reaches_connected

connection_ssh_connect_failure_reaches_error

connection_auth_failure_does_not_start_provisioning

connection_device_detection_failure_closes_ssh

connection_provision_failure_does_not_create_tunnel

connection_verify_failure_is_recoverable

connection_tunnel_failure_does_not_start_rdp

connection_rdp_failure_closes_new_tunnel

connection_disconnect_from_each_intermediate_state_is_safe

connection_cancel_is_idempotent

connection_retry_after_recoverable_error_restarts_from_correct_stage

connection_unrecoverable_error_does_not_loop_forever
B. State convergence

尤其重要：

connection_never_remains_connecting_after_terminal_failure

connection_all_failure_paths_converge_to_known_state

connection_duplicate_disconnect_is_noop

connection_duplicate_connect_is_rejected_or_coalesced

connection_stale_async_result_is_ignored_after_new_generation

最后这个非常重要。

例如：

Connect A
↓
Cancel
↓
Connect B

但 A 的 SSH callback 晚回来

绝对不能污染 B。

建议每次 connection attempt 有：

generation_id

或者：

attempt_id
2.3 Tunnel Tests
tunnel_allocates_unique_local_port

tunnel_port_collision_retries_with_new_port

tunnel_process_alive_but_socket_dead_is_unhealthy

tunnel_socket_alive_but_child_process_dead_is_unhealthy

tunnel_disconnect_only_closes_owned_process

tunnel_stale_pid_is_not_treated_as_valid

tunnel_reconnect_replaces_stale_tunnel

tunnel_two_sessions_never_share_credentials

tunnel_two_sessions_never_share_process_handle

tunnel_shutdown_is_idempotent

特别建议把当前：

process alive == tunnel alive

这种判断彻底禁止。

health 至少应该是：

process alive
AND
local port accepts connection
2.4 Device Identity Tests
identity_prefers_machine_id_when_available

identity_falls_back_to_serial_when_machine_id_missing

identity_falls_back_to_generated_fingerprint_when_both_missing

identity_same_machine_id_different_ip_is_same_device

identity_same_ip_different_machine_id_is_not_same_device

identity_serial_survives_machine_id_change

identity_duplicate_machine_id_with_different_serial_is_conflict

identity_normalization_is_stable
2.5 Credential Isolation
credentials_are_session_scoped

credentials_are_never_logged

credentials_are_never_serialized_into_diagnostics

credentials_are_never_present_in_process_argv

sudo_password_is_only_written_to_stdin

session_a_credentials_never_reach_session_b

saved_credentials_require_matching_device_identity
2.6 Diagnostics
diagnostics_redacts_password

diagnostics_redacts_sudo_password

diagnostics_redacts_private_key

diagnostics_redacts_token_like_values

diagnostics_preserves_non_sensitive_context

diagnostics_events_are_chronologically_ordered

diagnostics_each_failure_has_error_code

diagnostics_contains_attempt_id

diagnostics_contains_session_id

diagnostics_contains_connection_stage

建议以后明确：

任何用户可见失败
必须有 ErrorCode

例如：

SSH_AUTH_FAILED
SSH_HOST_KEY_CHANGED
PROVISION_STEP_FAILED
TUNNEL_START_FAILED
RDP_CONNECT_FAILED
RDP_ENGINE_CRASHED
2.7 Multi-session
multi_session_connect_two_devices_independently

multi_session_disconnect_a_does_not_affect_b

multi_session_reconnect_a_does_not_recreate_b_tunnel

multi_session_active_focus_routes_to_correct_session

multi_session_clipboard_routes_only_to_focused_session

multi_session_ime_routes_only_to_focused_session

multi_session_resize_only_updates_target_session

multi_session_error_isolation
2.8 Frontend Zustand / React

如果使用 Vitest：

session_store_add_session

session_store_remove_session

session_store_active_tab_switch

session_store_stale_event_is_ignored

session_store_state_snapshot_replaces_atomicly

session_store_error_does_not_mutate_other_session

session_store_disconnect_preserves_device_record
2.9 Provisioning Contract
provision_jsonl_each_line_is_valid_json

provision_jsonl_sequence_is_monotonic

provision_jsonl_has_run_id

provision_jsonl_has_schema_version

provision_step_check_before_apply

provision_step_apply_only_when_required

provision_step_verify_after_change

provision_second_run_is_noop

provision_partial_failure_can_resume

provision_unknown_step_is_forward_compatible

provision_stderr_does_not_corrupt_jsonl
2.10 Updater
update_valid_package_reaches_committed

update_invalid_signature_never_installs

update_interrupted_download_preserves_current_version

update_interrupted_extract_preserves_current_version

update_interrupted_after_backup_recovers_previous_version

update_interrupted_after_swap_recovers_on_next_launch

update_first_launch_crash_rolls_back

update_health_success_commits_new_version

update_rollback_is_idempotent

update_recovery_after_double_crash_is_safe
2.11 Device Trust
trust_first_seen_host_key_creates_tofu_record

trust_same_host_key_same_machine_id_is_trusted

trust_same_ip_changed_host_key_blocks_auto_connect

trust_changed_host_key_same_machine_id_requires_confirmation

trust_changed_machine_id_same_serial_is_reenrollment_candidate

trust_changed_machine_id_changed_serial_is_new_device

trust_same_host_key_changed_machine_id_is_identity_conflict

trust_conflict_never_auto_uses_saved_password

trust_replaced_host_key_revokes_old_binding
2.12 文件级变更

建议：

apps/desktop/src-tauri/
├── crates/
│   └── jr-test-support/
│       ├── src/
│       │   ├── fake_clock.rs
│       │   ├── fake_ssh.rs
│       │   ├── fake_tunnel.rs
│       │   ├── fake_rdp.rs
│       │   ├── fake_keychain.rs
│       │   ├── fake_fs.rs
│       │   ├── fault.rs
│       │   ├── recorder.rs
│       │   └── scenario.rs
│       └── Cargo.toml
│
├── tests/
│   ├── connection_scenarios.rs
│   ├── multi_session.rs
│   ├── diagnostics.rs
│   ├── provisioning_contract.rs
│   ├── updater_recovery.rs
│   └── device_trust.rs
│
scripts/
└── test/
    ├── all.sh
    ├── provisioning-matrix.sh
    ├── updater-fault-matrix.sh
    └── linux-vm-smoke.sh
2.13 本阶段完成即停

做到：

Pure tests
+
Fake integration
+
Ubuntu VM provisioning
+
macOS FakeRdp lifecycle

全部稳定即可停。

不要继续追：

JetPack-specific
RDP FPS
GPU acceleration
真实 clipboard latency
真实 IME edge cases

这些留给真机阶段。

2.14 本地验收命令

建议最终形成统一入口：

pnpm test
cargo test --workspace
./scripts/test/provisioning-matrix.sh
./scripts/test/updater-fault-matrix.sh

最后：

./scripts/test/all.sh

目标：

all.sh

✓ rust unit
✓ frontend unit
✓ connection scenarios
✓ provisioning contract
✓ updater fault matrix
✓ trust matrix
✓ macOS session lifecycle
3. TOP2 — Provisioning V2

这是我建议本阶段第二优先级最高的事情。

核心原则：

不重写 bootstrap 能力，只重写 bootstrap 的“契约”。

3.1 输出格式：明确选 JSON Lines

我的建议：

JSONL

不要自己设计：

STEP|XRDP|SUCCESS

这类 line protocol。

原因是你马上就会需要：

error code
before
after
duration
step version
restart requirement
retryability

最终行协议一定又变成半个 JSON。

JSONL 正好同时满足：

Streaming
Structured
Human-debuggable
Shell-friendly
Rust-friendly
Backward-compatible

例如：

{"schema":"jr.provision.event/v1","seq":1,"run_id":"01JXYZ","type":"run_started","provisioner_version":"2.0.0"}
{"schema":"jr.provision.event/v1","seq":2,"run_id":"01JXYZ","type":"step_started","step_id":"xrdp.package","step_version":1}
{"schema":"jr.provision.event/v1","seq":3,"run_id":"01JXYZ","type":"step_result","step_id":"xrdp.package","step_version":1,"check":"needs_change","apply":"changed","verify":"passed","changed":true}
{"schema":"jr.provision.event/v1","seq":4,"run_id":"01JXYZ","type":"run_completed","status":"success","changed":true}
3.2 stdout 与 stderr 必须严格分离

JSONL mode 下：

stdout
=
protocol only

绝对不要：

Installing XRDP...

混进 stdout。

普通 apt / systemctl 输出：

stderr

或者内部捕获后：

{
  "type": "log",
  "level": "debug"
}

但第一版甚至不需要传所有日志。

3.3 不要设计一个 idempotent: true

这一点很重要。

幂等不是一个声明。

它应该由执行结果证明。

建议每个 step 的结果统一为：

{
  "step_id": "xrdp.config",
  "step_version": 2,

  "check": "satisfied",
  "apply": "not_run",
  "verify": "passed",

  "changed": false
}

状态允许：

check
satisfied
needs_change
unknown
apply
not_run
noop
changed
failed
verify
passed
failed
not_run

那么：

第二次执行：
check = satisfied
apply = not_run
changed = false
verify = passed

这才是真正可测试的幂等。

3.4 每个 step 的最小 contract

概念上：

check()
apply()
verify()

Shell 中可以：

step_xrdp_package_check
step_xrdp_package_apply
step_xrdp_package_verify

目录建议：

remote/
├── bootstrap.sh
└── provision/
    ├── runner.sh
    ├── protocol.sh
    ├── context.sh
    └── steps/
        ├── 010_detect_os.sh
        ├── 020_xrdp_package.sh
        ├── 030_xrdp_config.sh
        ├── 040_xorgxrdp.sh
        ├── 050_xfce.sh
        ├── 060_session.sh
        ├── 070_permissions.sh
        ├── 080_services.sh
        └── 090_verify.sh

编号只用于稳定执行顺序。

逻辑 ID 不要包含编号：

xrdp.package
xrdp.config
xorgxrdp.config
xfce.session
xrdp.permissions
services.xrdp
environment.verify
3.5 Step Version

每个 step 自己拥有：

step_version

例如：

JR_STEP_ID="xrdp.config"
JR_STEP_VERSION=3

为什么？

因为：

Provisioner 2.4.0

可能只修改：

xrdp.config

而不是整套 recipe。

Diagnostics 可以看到：

xrdp.config@3

非常有价值。

3.6 三种 Version 必须区分

不要混成一个版本号。

Protocol schema
jr.provision.event/v1

定义：

Rust 如何解析 JSON
Provisioner version
2.0.0

定义：

整套 provisioning 软件版本
Step version
xrdp.config@3

定义：

这个 step 的 desired state 发生过几次语义变化
3.7 是否需要保存 remote state？

可以保存。

但：

它只能是 cache，不能是真相。

建议：

/var/lib/jetson-remote/provision-state.json

例如：

{
  "last_successful_provisioner": "2.0.0",
  "last_success": "2026-09-08T...",
  "profile": "ubuntu-22.04",
  "steps": {
    "xrdp.config": 3,
    "xfce.session": 2
  }
}

但是下一次启动绝不能：

state.json says success
→ skip checks

必须：

check actual system

因为用户可能手动改配置。

3.8 向后兼容现有 bootstrap.sh

这是最重要的迁移设计。

不要立即改变旧调用语义。

建议：

bootstrap.sh

继续作为稳定入口。

第一阶段：

bootstrap.sh
        │
        ├── 无参数
        │
        │    Legacy Mode
        │
        │    保持当前输出 + exit code
        │
        │
        └── --protocol jsonl
             │
             └── Provisioning V2

也就是：

./bootstrap.sh

旧客户端完全不受影响。

新客户端：

./bootstrap.sh \
  --protocol jsonl \
  --run-id "$RUN_ID"
3.9 更保守的迁移方式

如果你害怕第一次就把原脚本拆坏，可以：

bootstrap.sh
bootstrap_legacy.sh
provision/runner.sh

初期：

bootstrap.sh no args
→ bootstrap_legacy.sh
bootstrap.sh --protocol jsonl
→ provision/runner.sh

等 V2 经过充分验证，再让：

legacy mode

也内部复用新 steps。

这比“一次性重构原脚本”安全很多。

我更推荐这个方案。

3.10 Error object

统一：

{
  "error": {
    "code": "XRDP_SERVICE_START_FAILED",
    "message": "xrdp service failed to reach active state",
    "retryable": true,
    "command_exit_code": 1
  }
}

前端禁止解析：

英文错误字符串

进行逻辑判断。

只能根据：

error.code
3.11 interruption / resume

第一版不需要：

从 step 57 的 shell instruction 继续

那么复杂。

只需要：

重新运行整个 runner
↓
之前已满足的 step
check=satisfied
↓
自动跳过 apply

这已经实现 resumable。

也就是：

resume by idempotency

而不是：

resume by program counter

后者不要做。

3.12 本阶段里程碑

达到以下状态就停：

旧 bootstrap 调用没有行为变化
新 --protocol jsonl 可以完整跑
JSONL parser 有 contract tests
连续执行 3 次：
第一次 changed
后两次 changed=false
随机在 step 后 kill
重新运行可以收敛
Ubuntu 20.04 / 22.04 / 24.04 VM
协议均工作

暂时不要针对：

JetPack BSP
NVIDIA desktop 特例

做新修复。

已有规则保留即可。

3.13 验收命令

例如：

./remote/bootstrap.sh

验证 legacy。

然后：

./remote/bootstrap.sh \
  --protocol jsonl \
  --run-id test-001 \
  | jq -c .

重复：

for i in 1 2 3; do
  ./remote/bootstrap.sh \
    --protocol jsonl \
    --run-id "idem-$i"
done

然后：

./scripts/test/provisioning-matrix.sh

最终要求报告：

Ubuntu 20.04   PASS
Ubuntu 22.04   PASS
Ubuntu 24.04   PASS

JSONL contract PASS
Idempotency    PASS
Resume         PASS
Legacy mode    PASS
4. TOP3 — Embedded Session Architecture

这一阶段不要继续“优化 FreeRDP”。

核心任务：

冻结边界。

让以后任何 FreeRDP bug：

不会传播到整个 App。
4.1 目标架构

建议收敛为：

React
  │
  │ Tauri IPC
  ▼
SessionService
  │
  ▼
RdpSessionManager
  │
  ├── SessionRegistry
  ├── FocusCoordinator
  ├── ClipboardCoordinator
  ├── RecoveryPolicy
  └── SessionState
           │
           ▼
      RdpEngine trait
           │
           ▼
    FreeRdpAdapter
           │
           ▼
     C FFI Bridge
           │
           ▼
       FreeRDP

Native View 独立：

RdpSessionManager
      │
      ▼
NativeSurfaceHost

而不是让：

FreeRDP bridge

同时管理：

Cocoa Window
session
tunnel
clipboard policy
retry
4.2 建议文件结构
src-tauri/src/
├── session/
│   ├── mod.rs
│   ├── manager.rs
│   ├── registry.rs
│   ├── model.rs
│   ├── state.rs
│   ├── recovery.rs
│   ├── focus.rs
│   └── clipboard.rs
│
├── rdp/
│   ├── mod.rs
│   ├── adapter.rs
│   ├── config.rs
│   ├── event.rs
│   ├── error.rs
│   └── freerdp/
│       ├── mod.rs
│       └── ffi.rs
│
├── platform/
│   └── macos/
│       ├── surface.rs
│       ├── input.rs
│       └── clipboard.rs
│
└── ipc/
    └── session.rs

C：

src-tauri/native/freerdp/
├── jr_freerdp_bridge.h
└── jr_freerdp_bridge.c
4.3 RdpAdapter / RdpEngine 应该收哪些 API

接口尽量窄。

建议：

trait RdpEngine {
    fn start(
        &self,
        config: RdpConnectConfig,
        surface: NativeSurfaceHandle,
        event_sink: Arc<dyn RdpEventSink>,
    ) -> Result<RdpEngineHandle, RdpError>;

    fn stop(
        &self,
        handle: RdpEngineHandle,
    ) -> Result<(), RdpError>;

    fn resize(
        &self,
        handle: RdpEngineHandle,
        size: DesktopSize,
    ) -> Result<(), RdpError>;

    fn set_focus(
        &self,
        handle: RdpEngineHandle,
        focused: bool,
    ) -> Result<(), RdpError>;

    fn send_input(
        &self,
        handle: RdpEngineHandle,
        event: RdpInputEvent,
    ) -> Result<(), RdpError>;

    fn set_clipboard(
        &self,
        handle: RdpEngineHandle,
        payload: ClipboardPayload,
    ) -> Result<(), RdpError>;

    fn health(
        &self,
        handle: RdpEngineHandle,
    ) -> RdpEngineHealth;
}

够了。

4.4 不要暴露 FreeRDP 类型

接口中禁止：

freerdp*
rdpContext*
CliprdrClientContext*
RdpgfxClientContext*
NSView*
CALayer*
void*

Native handle 应该包装：

struct NativeSurfaceHandle(...)

Opaque。

前端更不能知道。

4.5 Adapter Event

建议只暴露：

enum RdpEngineEvent {
    Connecting,

    Connected {
        desktop_size: DesktopSize,
    },

    Disconnected {
        reason: RdpDisconnectReason,
    },

    Error {
        code: RdpErrorCode,
        retryable: bool,
    },

    ClipboardOffer {
        formats: Vec<ClipboardFormat>,
    },

    ClipboardData {
        payload: ClipboardPayload,
    },

    Metrics {
        latency_ms: Option<u32>,
    },
}

第一阶段甚至：

Metrics

都可以不做。

4.6 SessionManager 应该拥有的东西

以下全部必须在 bridge 外：

session_id
device_id
attempt_id

SSH association
tunnel ownership

reconnect policy
retry/backoff

active tab
focus ownership

clipboard ownership

IME ownership

viewport

connection state

diagnostics timeline

error mapping

resource cleanup order
4.7 C Bridge 只允许拥有

C Bridge 最终应该接近：

FreeRDP context creation

FreeRDP context destruction

FreeRDP connection

FreeRDP event loop

channel initialization

RDPGFX callbacks

CLIPRDR serialization

input packet translation

drawing into provided native surface

returning protocol-level events

它应该是：

Dumb protocol adapter.

而不是 application controller。

4.8 必须移出 Bridge 的现有实现细节

如果现在存在以下任何逻辑，逐步搬出去：

Tunnel creation
Tunnel PID tracking
SSH credentials
Device credentials
Keychain access

Reconnect timer
Reconnect count
Backoff

Tab ID
Active device
Focused device

Global clipboard ownership
Clipboard session routing

IME composition ownership
IME active-session policy

Window position
Window restoration
Window visibility

Frontend emit()
Tauri AppHandle

Diagnostics formatting

User-facing error string

Device history

Device identity

Provisioning state

尤其是：

Tauri AppHandle

最好不要出现在 FreeRDP C bridge 这一层。

4.9 NativeSurfaceHost

建议专门抽：

trait NativeSurfaceHost {
    fn create(&self, session_id: SessionId) -> Result<NativeSurfaceHandle>;

    fn attach(&self, session_id: SessionId);

    fn detach(&self, session_id: SessionId);

    fn resize(&self, session_id: SessionId, viewport: Viewport);

    fn destroy(&self, session_id: SessionId);
}

这个模块负责：

NSView
CALayer
main-thread dispatch
view geometry

FreeRDP 只拿：

NativeSurfaceHandle
4.10 React ↔ Rust IPC：只保留高层命令

建议前端最终只知道：

session_connect
session_disconnect
session_reconnect
session_set_active
session_resize
session_get_snapshot

如果 credentials 从 UI 初次输入：

session_connect

可以携带一次性 secret。

但 Rust 收到后立刻进入：

SecretString / Zeroizing

不要写进 store。

4.11 前端不应该存在的 IPC

禁止：

rdp_send_key

rdp_mouse_move

rdp_mouse_click

rdp_clipboard_channel

rdpgfx_frame

freerdp_resize

freerdp_reconnect

tunnel_restart

xrdp_channel_event

这些都是：

implementation detail leakage

4.12 React 接收什么 event

我建议不要发几十种 Tauri event。

直接：

session_snapshot_changed

内容：

{
  "session_id": "S1",
  "device_id": "D1",
  "state": "connected",
  "active": true,
  "viewport": {
    "width": 1440,
    "height": 900
  },
  "error": null
}

Rust 是：

Source of Truth.

React 是：

Projection.

这样你现在正确的：

单一 ConnectionState

理念可以继续保留。

4.13 内部状态建议再拆一层

UI：

ConnectionState

保持不变。

内部：

struct SessionRuntimeState {
    ssh: SshState,
    provision: ProvisionState,
    tunnel: TunnelState,
    rdp: RdpState,
}

然后：

derive_connection_state()

给 UI。

避免未来 enum 爆炸。

4.14 FakeRdpAdapter

TOP1 和 TOP3 结合的关键。

写：

FakeRdpAdapter

允许 scenario：

FakeRdpScenario::new()
    .connect_success()
    .disconnect_after(EventCount(5))

或者：

.connect_error(RdpErrorCode::TransportClosed)

这样可以跑：

1000 次

生命周期测试而无需 FreeRDP。

4.15 第一批 SessionManager 测试
session_connect_creates_exactly_one_engine

session_disconnect_stops_exactly_one_engine

session_engine_error_transitions_to_error

session_retry_creates_new_attempt_id

session_stale_engine_callback_is_ignored

session_resize_is_debounced

session_resize_only_affects_target_session

session_focus_is_exclusive

session_clipboard_owner_follows_focus

session_disconnect_releases_clipboard_ownership

session_disconnect_releases_native_surface

session_disconnect_releases_tunnel_after_rdp

session_cleanup_order_is_deterministic

session_second_disconnect_is_noop

session_multi_device_engine_handles_are_isolated

尤其：

cleanup order

建议明确：

stop RDP
↓
detach surface
↓
destroy surface
↓
close tunnel
↓
release secrets
↓
remove session

不要靠 Drop 顺序偶然成立。

4.16 本阶段完成即停

满足：

前端不存在 FreeRDP-specific IPC
SessionManager 可以完整跑 FakeRdpAdapter
FreeRDP bridge 不知道 tunnel/device/tab
所有异步 callback 带 session_id + attempt_id
multi-session lifecycle tests 全绿
真实 FreeRDP adapter 能编译

如果 Ubuntu VM XRDP 可用，再做：

单 session smoke connection

就停。

不要做：

FPS tuning
RDPGFX optimization
codec tuning
4.17 验收命令
cargo test -p jr-session
cargo test -p jr-session \
  session_
cargo test --workspace \
  multi_session

然后：

pnpm test

最后 debug 构建：

pnpm tauri build --debug

确保：

FFI adapter
NativeSurfaceHost
SessionManager

全部能链接。

5. TOP4-A — Updater Rollback

Updater 第一阶段不要直接拿真实 /Applications 做 fault injection。

应该先把 updater 核心做成：

filesystem transaction engine

然后在：

/tmp

中模拟完整 App。

5.1 状态机

建议：

Idle
 ↓
Downloading
 ↓
Downloaded
 ↓
Verifying
 ↓
Verified
 ↓
Staging
 ↓
Staged
 ↓
BackingUp
 ↓
Installing
 ↓
PendingHealth
 ↓
Committed

任意安装阶段失败：

RecoveryRequired
 ↓
RollingBack
 ↓
RolledBack

如果 rollback 也失败：

FailedSafe

但 FailedSafe 必须满足：

至少 current 或 backup 有一个完整 bundle。

5.2 Transaction Journal

Updater 不应该只靠内存状态。

每次更新生成：

transaction_id

例如：

01JUPDATEABC

写：

state.json
5.3 推荐文件布局

逻辑布局：

UpdaterRoot/
├── state.json
├── lock
│
├── transactions/
│   └── 01JUPDATEABC/
│       ├── manifest.json
│       ├── journal.jsonl
│       ├── downloaded/
│       ├── extracted/
│       └── health.json
│
├── backup/
│   └── 0.3.7/
│       └── Jetson Remote.app
│
└── logs/

生产中：

UpdaterRoot

可以放：

~/Library/Application Support/Jetson Remote/updater/

但是：

staged app 最终安装前，应该保证和目标 App 位于可安全 rename 的 filesystem。

不要假设：

/tmp

和：

/Applications

永远可以原子 rename。

生产实现应动态判断 destination volume。

5.4 state.json

例如：

{
  "schema": 1,
  "transaction_id": "01JUPDATEABC",

  "from_version": "0.3.7",
  "to_version": "0.4.0",

  "phase": "pending_health",

  "current_path": "/Applications/Jetson Remote.app",

  "backup_path": "...",

  "staged_path": "...",

  "started_at": "...",

  "health_committed": false
}

每个 phase 变化：

先持久化
再执行不可逆操作
5.5 安装顺序

建议：

Download
↓
Hash Verify
↓
Signature Verify
↓
Extract
↓
Bundle Integrity Check
↓
Stage on target filesystem
↓
Persist: BACKING_UP
↓
Move current → backup
↓
Persist: INSTALLING
↓
Move staged → current
↓
Persist: PENDING_HEALTH
↓
Launch new app
↓
New app self-check
↓
Write HEALTHY
↓
Persist: COMMITTED
↓
Delete old transaction
5.6 Health Check 不允许包含网络检查

不要：

能否连接 Jetson

作为 update health。

否则：

Jetson 关机
Wi-Fi 断了

会触发错误 rollback。

Health 必须只证明：

新版本 App 自己健康。

5.7 推荐 health criteria

新 App 必须完成：

✓ Tauri runtime initialized

✓ main window successfully created

✓ config database/store opened

✓ schema migration completed

✓ updater state readable

✓ session subsystem initialized

✓ native FreeRDP dynamic library loaded

✓ FreeRDP adapter self-test initialized

✓ keychain subsystem initialized

✓ frontend reached AppReady

完成后：

commit_health()

写：

{
  "transaction_id": "...",
  "app_version": "0.4.0",
  "status": "healthy"
}
5.8 不要单纯用“启动 5 秒没 crash”

因为：

window 没出来
session subsystem 崩了
数据库打不开

都有可能进程仍然活着。

最好使用：

明确 AppReady checkpoint
5.9 Fault Injection

统一环境变量：

JR_FAULT_INJECT

仅：

debug
test

编译启用。

生产 release 禁止。

Download
updater.download.before

updater.download.mid

updater.download.after
Extract
updater.extract.before

updater.extract.mid

updater.extract.after
Install
updater.backup.before

updater.backup.after

updater.swap.before

updater.swap.after
Launch
updater.launch.before

updater.launch.after
Health
app.start.before_runtime

app.start.after_runtime

app.start.before_window

app.start.after_window

app.start.before_health_commit

app.start.after_health_commit

fault 行为支持：

error
panic
process_exit

最有价值的是：

process_exit

真正模拟突然 kill。

5.10 关键恢复测试

每个 injection point：

执行 update
↓
kill
↓
重新启动 updater recovery

最终必须满足：

Current = new healthy version

或者：

Current = previous known-good

绝对不能：

Current missing

或者：

两个 bundle 都 incomplete
5.11 Updater 模块
src/updater/
├── mod.rs
├── state.rs
├── transaction.rs
├── journal.rs
├── download.rs
├── verify.rs
├── stage.rs
├── install.rs
├── recovery.rs
├── health.rs
└── fault.rs

关键是：

transaction.rs
recovery.rs

必须和：

Tauri UI

解耦。

5.12 本地 Sandbox

测试时：

/tmp/jr-updater-test/
├── Applications/
│   └── Jetson Remote.app
└── Application Support/
    └── Jetson Remote/

完全不用动真实 App。

5.13 本阶段完成即停

满足：

所有 fault point 都执行

并且：

任何 interruption 后重新启动 recovery
最终都有可启动版本

即可停。

暂时不要重新设计：

update server
release CDN
delta update
background download
5.14 本地验收
cargo test -p jr-updater

然后：

./scripts/test/updater-fault-matrix.sh

期待：

download.mid                 PASS
extract.mid                  PASS
backup.after                 PASS
swap.before                  PASS
swap.after                   PASS
launch.after                 PASS
before_health_commit         PASS
6. TOP4-B — Device Trust

这里需要区分两个完全不同的 identity：

SSH Host Identity

与：

Jetson Product Identity

前者是安全。

后者是产品逻辑。

不能混在一起。

6.1 SSH Identity

来自：

SSH host key

例如：

ssh-ed25519 SHA256:...

它回答：

这次 SSH server 是否还是我以前信任的那个 server？

6.2 Product Identity

来自：

machine-id
serial
fallback fingerprint

它回答：

这是不是 Jetson Remote 里面以前记录的那台设备？

6.3 TrustRecord

建议：

struct DeviceTrustRecord {
    logical_device_id: DeviceId,

    machine_id: Option<String>,
    serial: Option<String>,

    trusted_host_keys: Vec<HostKeyRecord>,

    known_addresses: Vec<KnownAddress>,

    first_seen_at: Timestamp,
    last_seen_at: Timestamp,
}

HostKey：

struct HostKeyRecord {
    algorithm: String,
    fingerprint: String,
    state: HostKeyState,
}
6.4 首次连接：TOFU

第一次：

IP
↓
SSH returns host key
↓
Unknown key
↓
TOFU candidate

用户正常点击 Connect。

产品不需要弹专业 SSH 对话框。

可以：

正在首次验证设备身份…

成功认证、读取 machine-id 后：

Host Key
+
machine-id
+
serial

共同保存。

6.5 以后正常连接
same host key
+
same product identity

直接：

Trusted

无需用户感知。

6.6 最重要规则
Host key mismatch 永远禁止自动重连。

即使：

IP 一样
设备名一样
machine-id 缓存一样

也不能静默接受。

因为 machine-id 是：

SSH 成功以后

才能拿到的。

Host key 是第一道安全 gate。

6.7 mismatch 时 UX 不要直接显示 SSH 术语

第一屏：

设备身份发生变化

Jetson Remote 检测到 192.168.1.20 的安全身份
与之前保存的记录不同。

这可能发生在：
• Jetson 重刷系统
• SSH 配置被重新生成
• 该 IP 被另一台设备使用

在确认前，已保存的密码不会自动用于连接。

按钮：

取消

验证这台设备

不要第一屏就：

Replace ECDSA SHA256 fingerprint?
6.8 “验证这台设备”是什么意思

允许：

temporary trust

只针对这一轮。

然后：

SSH authenticate
↓
read machine-id
↓
read serial

注意：

temporary trust 不能自动写永久 trust store。

之后再裁决。

6.9 冲突决策矩阵
情况 A
Host key changed
Machine-id same
Serial same

结论：

大概率 SSH host keys 被重建。

UX：

这看起来仍然是原来的 Jetson，
但它的 SSH 安全身份已重新生成。

[更新受信任身份]
[取消]

更新后：

旧 host key → revoked
新 host key → trusted
情况 B
Host key changed
Machine-id changed
Serial same

这非常像：

Jetson 被重新刷机

UX：

检测到这台 Jetson 可能已重新刷机。

硬件序列号与之前一致，
但系统身份和 SSH 身份都发生了变化。

[重新登记此设备]
[作为新设备添加]
[取消]

默认推荐：

重新登记此设备

但不要静默执行。

情况 C
Host key changed
Machine-id changed
Serial changed

或者 serial 不匹配。

结论：

大概率 IP 被另一台设备占用。

UX：

这似乎不是之前保存的那台 Jetson。

192.168.1.20 现在对应另一台设备。

[作为新设备添加]
[取消]

不要提供：

继续当成旧设备

作为主选项。

情况 D
Host key same
Machine-id changed
Serial same

可能：

系统重新生成 machine-id

允许重新登记。

情况 E
Host key same
Machine-id changed
Serial changed

非常异常。

可能：

镜像复制了 SSH host key

或者克隆系统。

这不能直接信任为旧设备。

显示：

设备身份冲突

建议：

作为新设备添加
6.10 Credentials 的裁决

这是安全关键。

Keychain 不能绑定：

192.168.1.20

应该绑定：

logical_device_id

而 logical_device_id 的 trust 状态发生冲突时：

Saved credential auto-use = disabled

直到用户确认：

Re-enroll

或者：

Add New Device
6.11 自动重连的规则

正常：

Trusted identity
↓
Auto reconnect

任何：

HOST_KEY_CHANGED
DEVICE_IDENTITY_CONFLICT

立即：

Auto reconnect suspended

不能：

不断每 5 秒重新试密码
6.12 Trust 模块
src/trust/
├── mod.rs
├── model.rs
├── host_key.rs
├── device_identity.rs
├── evaluator.rs
├── store.rs
└── decision.rs

核心：

evaluate_trust(
    previous,
    observed_host_key,
    observed_machine_id,
    observed_serial,
) -> TrustDecision

这应该是一个纯函数。

因此特别容易做单元测试。

6.13 TrustDecision

建议：

enum TrustDecision {
    Trusted,

    FirstSeen,

    HostKeyChangedNeedsVerification,

    ReEnrollSameHardware,

    NewDeviceAtKnownAddress,

    IdentityConflict,

    Blocked,
}

UI 只根据这个 enum。

不要自己组合：

if host_changed && machine_changed...

前端禁止做 trust business logic。

6.14 本阶段完成即停

达到：

所有 identity decision 都有 deterministic unit test
host key mismatch 禁止自动 reconnect
mismatch 禁止自动使用 saved credentials
Keychain binding 从 IP 转 logical_device_id

即可。

不需要真 Jetson。

可以使用本地：

OpenSSH server

生成不同 host key 做 integration test。

6.15 本地验收
cargo test -p jr-device-trust

额外：

./scripts/test/ssh-host-key-rotation.sh

脚本可以：

启动 sshd A
连接
记录

停止

更换 host key
启动 sshd B

验证：
HOST_KEY_CHANGED
7. 四块之间的依赖关系

不是简单：

1 → 2 → 3 → 4

实际是：

                 Reliability Harness
                        │
           ┌────────────┼────────────┐
           │            │            │
           ▼            ▼            ▼
 Provisioning V2   Session Arch   Updater/Trust
           │            │            │
           └────────────┼────────────┘
                        ▼
                 Release Gate

所以：

Harness 是唯一真正的前置依赖。

其他三块可以部分并行。

8. 推荐实施 SOP
Phase 0 — Freeze Contracts

第一件事情不是写功能。

先定义：

SessionId
AttemptId
DeviceId
RunId

ErrorCode

Event envelope

FaultInjector API

例如所有长期异步操作必须拥有：

session_id
attempt_id

所有 provisioning：

run_id

所有 updater：

transaction_id

这四种 ID 会解决非常多异步竞态问题。

Phase 1 — Reliability Harness Foundation

优先落：

FakeClock
FakeRdpAdapter
FakeTunnel
EventRecorder
FaultInjector

同时建立：

./scripts/test/all.sh

哪怕第一天只有：

8 tests

也没有关系。

关键是所有后续工作：

必须进入这套 Harness。

Phase 2A — Provisioning V2

可以由一个 Agent/分支独立进行。

主要改：

remote/bootstrap.sh
remote/provision/*
src/provision/*

依赖：

Harness

不依赖：

Session Architecture
Updater
Phase 2B — Embedded Architecture

与 Provisioning V2 并行。

主要改：

src/session/*
src/rdp/*
native/freerdp/*
src/platform/macos/*
src/ipc/session.rs

第一目标不是改 FreeRDP。

而是：

FakeRdpAdapter

能把所有 session tests 跑通。

Phase 2C — Device Trust

也可以并行。

它基本是纯逻辑。

甚至可以非常早完成。

主要：

src/trust/*

然后再接入现有：

SSH connect pipeline
device history
Keychain
Phase 3 — Updater Rollback

我会稍微晚于前三块。

原因不是技术依赖。

而是 updater 的 filesystem fault testing 需要已经有：

FaultInjector
Test filesystem
EventRecorder

因此 Harness 稳定之后做更舒服。

9. 如果你有多个 Agent，可以这样分
Agent A — Reliability Core

负责：

jr-test-support
ErrorCode
EventRecorder
FaultInjector
ScenarioRunner
CI scripts

尽量不动业务代码。

Agent B — Provisioning

负责：

bootstrap compatibility shim
JSONL protocol
step runner
Ubuntu tests
Agent C — Session

负责：

RdpSessionManager
RdpEngine
FakeRdpAdapter
FreeRDP bridge boundary
NativeSurfaceHost
Agent D — Trust + Updater

先：

DeviceTrust

再：

Updater transaction
rollback

因为两者都偏：

state machine
persistent state
recovery
10. 推荐合并顺序

即使并行开发，也不要随机 merge。

推荐：

1. test-support / common contracts

2. Device Trust pure core

3. Provisioning V2 protocol

4. SessionManager + Fake Adapter

5. Real FreeRDP adapter migration

6. Updater transaction core

7. Updater fault recovery

8. Full regression

为什么 Trust 可以这么早？

因为它：

代码面小
依赖少
风险相对低

可以快速建立“纯函数决策 + table-driven tests”的范式。

11. 每个阶段都执行的 Gate

禁止：

“功能写完后统一补测试”

而是每个模块都要求：

Contract
↓
Fake
↓
Implementation
↓
Fault
↓
Regression

例如 Provisioning：

JSONL schema tests
↓
fake events
↓
real shell runner
↓
kill halfway
↓
rerun

Session：

RdpEngine contract
↓
FakeRdpAdapter
↓
SessionManager
↓
disconnect fault
↓
FreeRDP implementation

Updater：

transaction state machine
↓
fake filesystem
↓
sandbox install
↓
process kill
↓
recovery
12. 建议增加统一 Reliability 命令

最终开发者不应该记：

12 条命令

增加：

pnpm reliability

内部执行：

frontend unit
rust unit
connection scenario
trust matrix
provision contract
updater faults

Linux VM 部分可：

pnpm reliability:linux

最终：

pnpm reliability:full
13. 本阶段 Release Gate

没有 Jetson 真机期间，我建议 v0.4 的工程 Gate 定义成：

Rust unit tests                 PASS

Frontend unit tests             PASS

Connection scenario tests       PASS

Multi-session isolation         PASS

Diagnostics redaction           PASS

Provision JSONL contract        PASS

Provision idempotency           PASS

Provision interruption resume   PASS

Ubuntu 20.04 contract           PASS

Ubuntu 22.04 contract           PASS

Ubuntu 24.04 contract           PASS

RdpSessionManager fake tests    PASS

FreeRDP adapter build           PASS

Updater fault matrix            PASS

Device trust matrix             PASS

Debug macOS app build           PASS

然后明确标记：

PENDING REAL-HARDWARE GATES

JetPack 5 real-device
JetPack 6 real-device
JetPack 7 real-device
power-cycle
long session
real clipboard
real IME
multi-Jetson

这样工程事实非常清楚。

14. 真机回来以后，只补这一层

这阶段做好以后，未来有真机不要重新“全面测试”。

直接把同一套 Harness 接上真实设备：

JR_TARGET=jp5-a
pnpm reliability:hardware
JR_TARGET=jp6-a
pnpm reliability:hardware
JR_TARGET=jp7-a
pnpm reliability:hardware

Hardware adapter 只替换：

FakeSsh
FakePower
FakeRdp

为：

RealSsh
SmartPlug / manual power
RealFreeRDP

上层 Scenario 不需要重写。

例如今天写：

scenario_reboot_then_reconnect()

现在：

Fake reboot

以后：

真实 Jetson reboot

Scenario 本身保持一致。

这才是 Reliability Harness 最大的长期收益。

15. TOP1–4 最终“完成即停”标准
TOP1 Reliability Harness

停在：

确定性状态机和 failure scenario 都可以自动跑

不追求硬件覆盖率。

TOP2 Provisioning V2

停在：

JSONL
step-based
idempotent
resumable
legacy-compatible

不要重写成 Rust Agent。

TOP3 Embedded Architecture

停在：

SessionManager 接管 lifecycle
Bridge 退化成 FreeRDP adapter
Fake adapter 测试完整

不要优化视频性能。

TOP4 Updater + Trust

停在：

任何本地 update interruption 都能恢复 known-good

任何 identity mismatch 都不会静默信任

不要增加复杂 update backend。

16. 我认为最关键的一个架构原则

这轮重构时建议在 AGENTS.md / 架构文档直接写成硬规则：

任何外部系统都必须通过 Adapter 接入核心状态机。

也就是：

SSH       → SshAdapter
Tunnel    → TunnelAdapter
Provision → ProvisionAdapter
RDP       → RdpEngine
Keychain  → SecretStore
UpdaterFS → UpdateFilesystem
Clock     → Clock

核心 orchestration：

永远不直接调用：
process::Command
FreeRDP C API
NSView
Keychain API
filesystem rename
system clock

这样才能做到：

Production Adapter
        ↕
Fake Adapter

一键替换。

如果这一条真正落下来，即使现在没有任何 Jetson 真机，你也可以把整个产品的大部分可靠性问题变成普通的确定性软件测试问题。

这就是我认为 v0.4 Reliability Release 最值得留下的工程资产。

如果让我进一步压缩成一个可直接给 Codex/Pi 执行的顺序，我会让它先只完成 jr-test-support + Connection Scenario Harness + ErrorCode/AttemptId，并设置硬 Gate 禁止碰 Provisioning/FreeRDP/Updater 业务实现；这个底座合并后，再让 3 个 Agent 分别跑 Provisioning、Session、Trust/Updater。这样最不容易出现“4 个 Agent 同时重构四套不同抽象”的情况。