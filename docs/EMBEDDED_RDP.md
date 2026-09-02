# EMBEDDED_RDP — 嵌入式 FreeRDP 技术 Spike 记录

> Phase 4B：把桌面从 `sdl-freerdp` sidecar 升级为 `libfreerdp` + `.app` 内 Native Surface。
> 本文件是 Spike 的事实记录与「不要猜 API，读 headers/参考客户端」的结论，供实现与 review 对照。

## 1. 结论（已真机验证）

**`libfreerdp`（经 C bridge）直接连真实 Jetson `.164` 并收到真实 framebuffer，无需 `sdl-freerdp`、无需原生窗口。** `embedded_rdp_probe` headless 输出：

```
FreeRDP bridge version: 3.31.0
gdi_init_ex: Local framebuffer PIXEL_FORMAT_BGRA32  ← GDI 渲染管线就绪
[probe] connected
[probe] frame#12: 1280x720 stride=5120             ← 5120 = 1280×4 (BGRA32)，正确
disp 动态虚拟通道已加载（dynamic resolution 可用）
```

`license BB_ERROR_BLOB` 为 XRDP 非致命杂讯（KNOWN_ISSUES KI-011），连接照常成功。

## 2. FreeRDP 3.31.0 客户端 API 事实（读 headers 所得，非猜）

- **入口点**（`freerdp/client.h`）：`RDP_CLIENT_ENTRY_POINTS` == `rdp_client_entry_points_v1`，仅 10 字段：
  `Size`(=sizeof)、`Version`=`RDP_CLIENT_INTERFACE_VERSION`(1)、`settings`(可 NULL)、`GlobalInit/GlobalUninit`、`ContextSize`、`ClientNew`(freerdp*,rdpContext*)、`ClientFree`、`ClientStart`、`ClientStop`。
  - `freerdp_client_context_new(&ep) -> rdpContext*`；`context->instance/settings/gdi/input/update` 皆在 struct 内（offset 0/33/38/39/40）。
- **证书/Auth/生命周期回调都在 `freerdp* instance` 结构上（不是 rdpSettings）**：
  `instance->PreConnect/PostConnect/PostDisconnect`、`VerifyCertificateEx`(offset 66)、`VerifyChangedCertificateEx`(67)、`AuthenticateEx`(69)。
  - `pVerifyCertificateEx(freerdp*, host, port, common_name, subject, issuer, fingerprint, flags)` 返回 **1=接受+存储 / 2=仅本次 / 0=拒绝**（TOFU）。
  - `pVerifyChangedCertificateEx(... new_fingerprint, old_subject, old_issuer, old_fingerprint, flags)` 变化返回 0 拒绝。
  - **order：`freerdp_client_context_new` 先设 common 默认 cert/auth 回调，再调 `ClientNew`**（client/common/client.c:86-89 先、:126 `IFCALL(ClientNew)` 后）→ 我在 ClientNew 里覆盖即可生效。
- **connect/事件循环**：`freerdp_client_start(context)` → `freerdp_connect(instance)` → `while(!freerdp_shall_disconnect_context(context)) { freerdp_get_event_handles(context, handles, n); WaitForMultipleObjects(...); freerdp_check_event_handles(context); }` → `freerdp_disconnect`。跨线程断开用 `freerdp_abort_connect_context(context)`。
- **GDI 渲染**：`gdi_init(instance, PIXEL_FORMAT_BGRA32)`（PostConnect 里）→ `context->gdi` → `begin_paint`/`end_paint`/`desktop_resize` 挂在 `context->update->BeginPaint/EndPaint/DesktopResize`；framebuffer 在 `gdi->primary_buffer`，尺寸 `gdi->width/height`，`stride`，脏区在 `gdi->primary->hdc->hwnd->invalid`（spike 暂用 full-frame）。
- **settings**：`freerdp_settings_set_string/uint32/bool(settings, FreeRDP_ServerHostname/ServerPort/Username/Password/DesktopWidth/DesktopHeight/ColorDepth/AutoReconnectionEnabled, ...)`。
- **输入**：`freerdp_input_send_mouse_event(context->input, PTR_FLAGS_*, x, y)`；`freerdp_input_send_keyboard_event(input, KBD_FLAGS_DOWN|RELEASE|EXTENDED, scancode)`；`freerdp_input_send_focus_in_event(...)`。
- **像素格式**：`PIXEL_FORMAT_BGRA32`（`freerdp/codec/color.h`；与 Phase 4A sidecar 观测量一致）。

## 3. 参考实现

`client/Sample/tf_freerdp.c`（FreeRDP 3.31.0 tag，Apache-2.0）为最小客户端范本（sparse clone 到 `/tmp/freerdp-ref`）。`tfContext = { rdpClientContext common; ... }` → 自定义 context 首成员必须是 `rdpClientContext`（其首成员是 `rdpContext`），`ContextSize = sizeof(自定义)`。我们的 bridge 用 `jr_context_t { rdpClientContext common; jr_session_t* session; }` 回指针。

## 4. C Bridge 结构

```
native/freerdp_bridge/
  bridge.h           # 稳定 C 接口（opaque jr_session_t、jr_session_callbacks_t/证书/参数、jr_view_*）
  bridge.c           # FreeRDP 客户端 + gdi + 证书 TOFU + 输入（阻塞 event loop，跑 worker 线程）
  macos_view.m       # NSView 子类（AppKit 主线程），framebuffer blit + 事件透传（Phase 4B-2 接）
src-tauri/
  build.rs           # cc + pkg-config(freerdp3/client3/winpr3) + AppKit/CoreGraphics framework
  src/rdp/ffi.rs     # Rust FFI 声明（raw ptr 只在此 + session.rs）
  src/bin/embedded_rdp_probe.rs  # headless 诊断（连真机收 framebuffer）
```

- 线程模型：每 session 一个 FreeRDP worker 线程（阻塞 loop）→ `on_frame_updated` 回调在此线程读 `primary_buffer` → Rust 侧双缓冲 → `dispatch_async(主队列)` 通知主线程 NSView 呈现（Phase 4B-2）。
- 帧不经过 JS：React 只传 geometry，`ResizeObserver` → `jr_view_set_frame`。

## 5. 关键坑

1. **`cc` + pkg-config 构建**：`.file()` 路径相对 crate root（`src-tauri`），故用 `../native/...`；`probe_library` 同时 emit include/link；`.m` 用 `-fobjc-arc`，clang 自动识别 `.m` 为 ObjC。
2. **stale `.a` 未重编**：快速连续编辑 `build.rs` 会换 OUT_DIR hash 导致 .a 未被发现为 stale；`touch build.rs` 强制重编即可（cc 的 `rerun-if-changed` 偶发 miss）。
3. **opaque handle**：Rust 侧 FFI opaque 类型用 `#[repr(C)] pub struct jr_session { _private: [u8; 0] }`（`enum {}` + `#[repr(C)]` 会 E0084）；跨线程传 raw ptr 用 `usize` 载体（probe，诊断用）。
4. **ffi wrapper 无必要 `Setting`**：`PROBE` 用 `SessionHandle` + `unsafe impl Send`；正式 `RdpSession` 会在 worker 线程封好。
5. `-mmacosx-version-min=11.0`（Rust 默认）vs brew freerdp dylib built for 26.0 → 仅 linker warning，dev 无碍；release 自包含打包（4B finalization）解决。

## 6. 4B-2 Native View 补充（2026-09-01 验证）

- **渲染字节序（已修正为 BGRX32）**：`gdi_init` 用 `PIXEL_FORMAT_BGRX32`（B,G,R,X 字节序）↔ CoreGraphics `kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst`（=B,G,R,X）。这是 FreeRDP 官方 Mac 客户端的组合（`client/Mac/MRDPView.m`）。之前的 `PIXEL_FORMAT_RGBA32` + `kCGImageAlphaNoneSkipLast|Little` 是**错配**（G↔B 互换 + alpha 当红通道 → 洋红偏色）；字节序错只会偏色不纯黑，纯黑=缓冲区全零。真机确认 gdi log 打印 `Local framebuffer format PIXEL_FORMAT_BGRX32`。
- **NSView 渲染**：`CALayer.contents = CGImage`（`CGDataProviderCreateWithData` + release 回调 free 拷贝），`contentsGravity = kCAGravityResizeAspect`；**需 `#import <QuartzCore/QuartzCore.h>` + `-framework QuartzCore`**（`kCAFilterLinear`/`kCAGravityResizeAspect` 在 QuartzCore，不在 AppKit/CoreGraphics）。
- **线程**：所有 `jr_view_*` 经 GCD 派发到主线程（`run_on_main`）；`present_buffer` **先 memcpy（工作线程）再 dispatch_async 主线程建 CGImage**（因为 FreeRDP 会复用 primary_buffer）。
- **cc rerun-if-changed 用相对 `../` 路径不可靠** → build.rs 里对 native 源文件 `canonicalize()` 成绝对路径再 `.file()`；header 显式 `cargo:rerun-if-changed`。否则快速连续改 C 不会触发重编（stale .a）。
- **端到端验证**：`cargo tauri dev` → embedded 引擎连真机 `.164` → sesman 日志 `reconnected session display :10.0`（与 app 内 FreeRDP 日志同时刻），桌面渲染进主窗口。

## 7. 黑屏根因与修复（2026-09-01，4B-2 Gate Reopen）

**纯黑 ≠ 字节序问题**（字节序错只会 G↔B 互换/偏色）。纯黑 = `primary_buffer` 全零 = 桌面帧从未合入。

- **根因**：`gdi_init()` 只注册 legacy GDI 回调（BitmapUpdate / 绘图指令 / `SurfaceBits`），**不注册 RDPGFX 图形管线**（`gdi_StartFrame/EndFrame/SurfaceCommand/CreateSurface/MapSurfaceToOutput/UpdateSurfaces`）。这些必须由 `gdi_graphics_pipeline_init(gdi, gfx)` 注册，而该调用处于 `freerdp_client_OnChannelConnectedEventHandler` 内、当 RDPGFX 动态通道（`RDPGFX_DVC_CHANNEL_NAME`）连接时触发。我们的 bridge 从未订阅 PubSub 通道事件 → GFX 帧被静默丢弃 → `EndPaint` 永不触发、`primary_buffer` 全零。
- **修复**（`native/freerdp_bridge/bridge.c`）：`jr_pre_connect` 中订阅默认处理器：
  ```c
  PubSub_SubscribeChannelConnected(instance->context->pubSub,
                                   freerdp_client_OnChannelConnectedEventHandler);
  PubSub_SubscribeChannelDisconnected(instance->context->pubSub,
                                      freerdp_client_OnChannelDisconnectedEventHandler);
  ```
  需 `#include <freerdp/event.h>` + `<winpr/collections.h>`。post_disconnect 反订阅。GFX 与 legacy 都写入同一个 `gdi->primary_buffer`，所以一个 `EndPaint`→blit 覆盖两路。
- **诊断（DEV-only，`bridge.c` `jr_diag_frame`）**：`JR_RDP_DIAG=1` 每帧采样 2048 点打印 `nnz/rgb_nonzero/min/max/hash/首中末像素`；`JR_RDP_DUMP_FRAME=1` 落盘 `/tmp/jetson-remote-frame*.pppm`。probe 主循环同时直接轮询 `jr_session_get_framebuffer`（与 `EndPaint` 解耦，能抓到 GFX 路径写入的内容）。
- **验证**：隧道（绕过 macOS 本地网络权限）→ 连接成功、`Local framebuffer PIXEL_FORMAT_BGRX32`、`primary_buffer` 非黑（`nnz=4096/4096`）。

**附：本机两处环境坑（详见 KNOWN_ISSUES KI-013/014/015）**
1. macOS「本地网络」隐私(TCC) 拦 unsigned 二进制直连（`nc`/`ssh` 是 Apple 签名放行，Homebrew python/cargo 未签名 → `No route to host`/`Couldn't get socket ip address`）。已加入 `src-tauri/Info.plist` 的 `NSLocalNetworkUsageDescription`（bundle 内已确认注入），app 首次启动会弹授权；前端加了有界自动重连（允许后自动 retry）。
2. 设备 xrdp TLS 私钥 `key.pem`（snakeoil）权限：`xrdp` 用户不在 `ssl-cert` 组 → `Permission denied`，已 `adduser xrdp ssl-cert` 修复。
3. 新建 Xorg 会话窗口管理器段错误（`exit 139`）：A/B 已排除 GL 合成器（`use_compositing=false` 后 GL 警告消失但 segfault 仍在），真根因待 gdb 定位；重连既有 `:10` 不受影响。
## 8. 输入透传（2026-09-01，4B-2 input 接线）

**症状**：桌面能渲染但鼠标/键盘无效——`JRView` 只有 blit 没有事件处理，`jr_session_send_*` 无人调用。

**接线**：
- `session.rs`：`launch` 后 `view.attach_input(Some(session))`；`shutdown` 先 `attach_input(None)`（同步 `dispatch_sync` 回主线程）再 disconnect/join/destroy —— 保证主线程事件 handler 不可能在 session 销毁后触碰它（事件与 detach 同在主线程，天然串行）。
- `macos_view.m`：`JRView` 成为 first responder（`acceptsFirstResponder`/`acceptsFirstMouse`/`viewDidMoveToWindow`），`updateTrackingAreas` 开 `NSTrackingMouseMoved|ActiveInKeyWindow|InVisibleRect`。
- 鼠标：左/右/中 down/up/drag/moved → 语义函数 `jr_session_send_mouse_move/button`（bridge.c 翻译成 `PTR_FLAGS_*`，按住状态用 bitmask 随 move 带出）；`scrollWheel` → `jr_session_send_mouse_wheel`（离散 notch×120=WHEEL_DELTA，trackpad 精确 delta×3，clamp ±511，`WheelRotationMask`）。
- 键盘：`keyDown/keyUp` 用 Apple vk→XT scancode 表（`jr_scancode_for_vk`，含 keypad/F 键/方向键 ext）；`isARepeat`→`KBD_FLAGS_DOWN`（FreeRDP 3 语义：无 RELEASE=press，DOWN=autorepeat）；`flagsChanged` 按 modifier bit diff 发左右 shift/ctrl/alt/cmd/caps 的 down/up（右侧走 `KBD_FLAGS_EXTENDED`）。
- 坐标：layer 是 aspect-fit（letterbox），`mapPoint` 用 `jr_session_get_size` 的桌面尺寸算 scale/offset，x 直取、y 翻转（AppKit 左下 → RDP 左上），clamp 到桌面边界。

**验证**：dev tunnel 下真机连接，鼠标点击/拖动/滚轮、键盘输入均生效（2026-09-01）。

## 9. D-Bus 会话隔离（2026-09-01，单实例应用串屏修复）

**症状**：远程桌面点 gnome-terminal，窗口开在物理显示器侧。
**根因**：xrdp 会话没起私有 session bus，`DBUS_SESSION_BUS_ADDRESS` 共享 `/run/user/$UID/bus`；D-Bus 单实例应用（gnome-terminal-server 先跑在物理 `:1`）把开窗请求路由到 server 所在 display。
**修复**：`~/.xsession` = `eval $(dbus-launch --exit-with-session)` + `exec startxfce4`（bootstrap.sh 幂等同步）。每会话独立总线后，单实例应用在各自会话内起 server。需重建会话生效（注销/kill Xorg :10 后重连）。

## 10. 文本剪贴板（CLIPRDR）双向接线（2026-09-02 验证）

**接线三步（全部必须）**：
1. `RedirectClipboard=TRUE`（连接前设置）。
2. 通道就绪信号二选一也可能先后到达，都要接：`ChannelConnected` PubSub 事件（`e->pInterface` 即 `CliprdrClientContext*`）；接口查询兜底 `jr_clip_ensure()`（`freerdp_channels_get_static_channel_interface(context->channels, "cliprdr")`，通道 OPEN 完成后才非空）。Mac 粘贴板 0.5s 轮询老板调用时顺带重试。
3. 客户端先发 ClientCapabilities（general set, version 2, len 12）；ServerCapabilities 只记录不回；MonitorReady 标记 ready 并宣告当前粘贴板格式列表。

**数据交换**：
- 远程→Mac：ServerFormatList → ClientFormatListResponse(OK) → ClientFormatDataRequest(13) → ServerFormatDataResponse → UTF-16LE→UTF-8 → 主线程写 NSPasteboard（changeCount 回环抑制）。
- Mac→远程：粘贴板变化 → ClientFormatList{CF_TEXT,CF_UNICODETEXT} → 远端粘贴时 ServerFormatDataRequest(13/1) → 应 UTF-16LE（带 null 终止）/ASCII。

**踩坑**：PubSub 回调第二参数语义——handler 收到的 `context` 是 `rdpContext*`（事件发布方传 `instance->context`），切不可当 `freerdp*` cast；签名错了读出的 session 是垃圾值且表现为"静默不接线"。
