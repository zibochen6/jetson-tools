# 远程桌面连接回归防护指南（长期维护）

> 本文档沉淀 Jetson Remote 开发过程中所有导致「无法远程连接 / 桌面不可操作」的真实事故与修复经验。
> **任何涉及连接链路的功能开发，合入前必须通过第 3 节回归清单。**
> 维护规则：每次新增一个连接相关事故或修复，同步追加到第 5 节并更新本文件日期。

---

## 0. 一句话原则

**远程桌面链路是「多环节串联 + 多处隐藏状态」的系统：任何一环的编码细节错误都表现为完全不相干的症状（黑屏、拖不动、点不中、连不上）。改代码前先画链路，改完用设备侧"地面真值"（xdotool/xinput/xclip）做闭环验证，不要只看 app 自己的日志。**

---

## 1. 链路全景与各环节脆弱点

```
前端 tauriService.ts (路由/端口)
  → SSH 控制面 russh (探活/检测/自举校验)
    → 设备 bootstrap.sh / ~/.xsession (会话环境)
      → xrdp-sesman → Xorg :N 会话 (XFCE + xfwm4 + xfce4-screensaver)
        ← 嵌入式 FreeRDP 客户端 (bridge.c: freerdp_connect + GDI)
          ← native view (macos_view.m: 渲染 + AppKit 输入转发)
            输入: AppKit 事件 → jr_session_send_* → RDP 输入 PDU → xrdp → xorgxrdp → X
            剪贴板: NSPasteboard ⇄ CLIPRDR 通道 ⇄ X 剪贴板
```

| 环节 | 主要脆弱点 |
|---|---|
| 前端路由 | dev tunnel 模式 (TCC)、端口覆盖、TOFU 信任流 |
| SSH 控制面 | 主机密钥信任库 (hosts.json)、密钥环密码解析 |
| 设备会话环境 | xfwm4 冻结、屏保挡窗、D-Bus 私总线、合成器 |
| 嵌入式 FreeRDP | 通道接口时序竞争、PubSub 回调 cast、capabilities 握手 |
| 输入转发 | **RDP 指针编码语义**（本仓库最大事故源） |
| 渲染 | RDPGFX 接线（黑屏根因之一）、分辨率与 letterbox |

---

## 2. 不可破坏的不变量（硬性规则）

### 2.1 RDP 指针输入编码表（xrdp 语义，血的教训）

xrdp 对 `TS_POINTER_EVENT` 的解析（v0.9.17 `xrdp/xrdp_wm.c::xrdp_wm_process_input_mouse`,**不是直觉上的"状态位"**）：

| 发送内容 | flags | xrdp 行为 |
|---|---|---|
| 按下 | `BUTTONn \| DOWN`（可带 MOVE 先定位） | 移动 + 按键按下 |
| **松开** | **纯 `BUTTONn`（无 DOWN、无 MOVE）** | 按键松开 |
| 移动/拖动 | **纯 `MOVE`（绝不能带按钮位）** | 仅移动；按住状态由服务端自己维护 |

**事故案例（KI-018）**：
- up 发 `MOVE\|BUTTON1` → xrdp 仍认为"按住"，永不松开 → 双击标题栏后窗口**黏住鼠标永不松手**、其余点击全部失效。
- 拖动时发 `MOVE\|BUTTON1` → 每个移动都被 xrdp 当"松开" → 拖拽窗口失效。
- 违反本表导致的症状：拖不动、黏窗、点击失灵、最大化无响应——全部与"编码"无关的外部症状。

**规则**：修改 `bridge.c` 的 `jr_session_send_mouse_*` 或 `macos_view.m` 事件处理器时，逐行对照此表；并用 3.1 节命令验证 press/release 配对。

### 2.2 剪贴板（CLIPRDR）四坑

1. **客户端先发 ClientCapabilities**（MS-RDPECLIP 规范+FreeRDP 官方客户端实践）；服务器回的 ServerCapabilities **只记录不回**（防 ping-pong）。
2. **通道接口时序竞争**：`freerdp_channels_get_static_channel_interface("cliprdr")` 在 PostConnect 时可能是 NULL（通道 OPEN 未完成）。接线必须双保险：① `ChannelConnected` PubSub 事件（`e->pInterface` 即上下文）；② 懒重试 `jr_clip_ensure()`（由 0.5s 粘贴板轮询顺带触发）。
3. **PubSub 回调 cast**：回调收到的 `context` 是 `rdpContext*`（发布方传 `instance->context`），**不是 `freerdp*`**。cast 错的症状是：session 读出垃圾值，一切"静默不接线"。
4. **线程**：CLIPRDR 回调在 FreeRDP worker 线程；NSPasteboard 读写必须在 AppKit 主线程（dispatch_sync），轮询定时器本身在主线程。

### 2.3 设备侧三状态坑（每个都会伪装成"app 的 bug"）

| 状态 | 症状 | 检测 | 修复（已在 .xsession/bootstrap.sh 固化） |
|---|---|---|---|
| xfwm4 冻结 | 窗口有标题栏但拖不动、最大化无响应；`xprop -set _NET_WM_STATE` 无反应 | `ps aux\|grep xfwm4` 活着但 EWMH 无响应 | `xfwm4 --replace`；会话级关闭 GL 合成器 |
| xfce4-screensaver 挡窗 | **所有**鼠标点击/拖拽失效；`xdotool getmouselocation` 全屏命中 1280x800 无名窗口 | 见 4.1 hit-test 扫描 | `pkill -x xfce4-screensaver`；autostart `Hidden=true` |
| D-Bus 共享总线 | gnome-terminal 等单实例应用窗口开到物理显示器侧 | `/proc/<session>/environ` 的 `DBUS_SESSION_BUS_ADDRESS` 相同 | `.xsession` 里 `dbus-launch` 起每会话私总线 |

### 2.4 其他不变量

- **分辨率 = 会话级**：会话分辨率在 xrdp 登录时确定（`session.rs desktop_size_for`），改窗口大小不会自动调；同用户重连会**复用旧会话**，改几何/`.xsession` 必须杀旧会话重建（`pkill -f '^/usr/lib/xorg/Xorg :10'`，**必须带锚点**，否则自杀——顺手杀掉包含同样字符串的自身 ssh 脚本）。
- **dev tunnel 模式**：macOS TCC 拦未签名二进制的 LAN 直连（KI-004）。`VITE_JR_SSH_PORT` 设了就必须 SSH/RDP 双平面走 127.0.0.1；改动 `tauriService.ts` 路由逻辑重看 KI-004 缓解设计。
- **RDPGFX 接线**：`pre_connect` 里的 `PubSub_SubscribeChannelConnected` 订阅 `freerdp_client_OnChannelConnectedEventHandler` 是黑屏根因（KI-013）——通道接线逻辑动它前必须理解。
- **杀会话用锚定模式**：`pkill -f "^/usr/lib/xorg/Xorg :10"`；裸 `pkill -f "Xorg :10"` 会匹配自身 ssh 命令行自杀。

---

## 3. 回归清单（任何连接相关改动合入前必做）

### 3.1 输入路径（必须全绿）

```bash
# A. 应用内部日志确认事件确实到达 handler
grep -E "jr-input" /tmp/tauri-dev.log | tail -5   # mouseDown/Dragged/Up 与移动应成对

# B. 设备侧抓 15s：press 与 release 必须配对（数量>0，且 release>0）★金丝雀★
#    （要求用户在 app 里点几下/拖一次，或见 C 合成注入）
DISPLAY=:10.0 timeout 15 xinput test 6 > /tmp/xev.log
grep -c "button press" /tmp/xev.log; grep -c "button release" /tmp/xev.log

# C. 合成事件闭环（不依赖用户操作）：CGEvent → app → RDP → X
/tmp/jrbounds                       # 拿 app 窗口 bounds
/tmp/jrdrag <W> 28 <dx> <dy> <ddx> <ddy> <dw> <dh>   # 注入拖拽
# 之后 A/B 应有记录；设备侧窗口几何应移动：
DISPLAY=:10.0 xwininfo -id <window> | grep "Absolute upper-left"

# D. 设备侧地面真值（XTEST，与我们的 RDP 路径对照）
DISPLAY=:10.0 xdotool mousemove <x> <y>; xdotool mousedown 1; \
  xdotool mousemove_relative -- 30 20; xdotool mouseup 1
```

**判定**：A/C 有事件、B 的 release>0、D 能拖窗 ⇒ 输入链路 OK。任一项失败即 STOP，对照 2.1 表。

### 3.2 剪贴板双向（必须全绿）

```bash
# 远程→Mac
ssh seeed@127.0.0.1 -p 2222 'printf "REMOTE_TEST" | DISPLAY=:10.0 xclip -selection clipboard &'
sleep 2; pbpaste                     # 期望 REMOTE_TEST

# Mac→远程
printf "MAC_TEST" | pbcopy
sleep 2
ssh seeed@127.0.0.1 -p 2222 'DISPLAY=:10.0 timeout 6 xclip -selection clipboard -o'
                                     # 期望 MAC_TEST
```

同时看 app 日志握手链：`grep "jr-clip" /tmp/tauri-dev.log` 应含「sending client capabilities → server capabilities received → monitor ready → announcing format list」。

### 3.3 连接流程

```bash
grep -E "jr-flow" /tmp/tauri-dev.log    # probe start→authenticated→detected ok→prepare start→rdp launch start
```

### 3.4 桌面可用性（视觉/操作抽查）

- [ ] 画面非黑屏且持续刷新（无 KI-013 回归）
- [ ] 打开一个窗口、点最大化、拖标题栏、最小化
- [ ] 双击标题栏不黏窗（KI-018 金丝雀）
- [ ] 桌面点击/右键正常（无屏保挡窗：`xdotool getmouselocation` 命中的是真实窗口而非全屏无名窗口）
- [ ] 字体大小正常（分辨率匹配窗口，无 letterbox 挤压）

### 3.5 构建验证

```bash
cd src-tauri && cargo check && cargo clippy
pnpm typecheck && pnpm lint && pnpm test
# 确认 native 改动真的进了二进制（strings 只找字符串字面量，用 nm）：
nm src-tauri/target/debug/jetson-remote | grep -c jr_session_set_clipboard_text
```

---

## 4. 调试工具速查表

### 4.1 设备侧（ssh -p 2222 seeed@127.0.0.1）

| 目的 | 命令 |
|---|---|
| 事件抓取（金丝雀） | `DISPLAY=:10.0 xinput test 6`（xrdpMouse）|
| 拖拽真相 | `xdotool mousemove/mousedown/mousemove_relative/mouseup` + `xwininfo -id <win> \| grep "Absolute upper-left"` |
| 全屏挡窗扫描 | `for 点位: xdotool mousemove X Y; xdotool getmouselocation`（对照当前窗口几何）|
| WM 存活 | `xprop -id <win> -f _NET_WM_STATE 32a -set _NET_WM_STATE "_NET_WM_STATE_MAXIMIZED_VERT,_NET_WM_STATE_MAXIMIZED_HORZ"` 后窗口应最大化 |
| 剪贴板真相 | `xclip -selection clipboard -o`（读）/ `… xclip -selection clipboard`（写） |
| 进程环境（会话归属） | `tr '\0' '\n' < /proc/<pid>/environ \| grep -E 'DISPLAY\|DBUS'` |
| xrdp 日志 | `/var/log/xrdp.log`、`~/.local/share/xrdp/xrdp-chansrv.<N>.log` |

### 4.2 Mac 侧

| 目的 | 路径/命令 |
|---|---|
| 粘贴板读 | `pbpaste` / 写 `pbcopy` |
| app 日志 | `/tmp/tauri-dev.log`（关键字：`jr-flow` 流程、`jr-input` 输入、`jr-clip` 剪贴板、`ERROR`/`WARN`） |
| 合成输入注入 | `/tmp/jrdrag`（CGEvent，需要 app 窗口 bounds=`/tmp/jrbounds`） |
| SSH 免交互 | `SSH_ASKPASS=/tmp/jr-askpass.sh SSH_ASKPASS_REQUIRE=force ssh ...` |

### 4.3 关键源码锚点

| 模块 | 文件 | 关键函数 |
|---|---|---|
| 输入 | `native/freerdp_bridge/bridge.c` | `jr_session_send_mouse_move/button`（2.1 表） |
| 输入 | `native/freerdp_bridge/macos_view.m` | `mouseDown/Dragged/Up`、`mapPoint`（letterbox 映射） |
| 剪贴板 | `bridge.c` | `jr_clip_wire/ensure`（2.2 四坑） |
| 剪贴板 | `macos_view.m` | `jr_mac_clip_set/get`、`jr_clipboard_sync_start` |
| 会话 | `src-tauri/src/rdp/session.rs` | `desktop_size_for`、attach/detach 顺序 |
| 前端路由 | `src/features/connection/tauriService.ts` | TUNNEL_MODE（2.4） |
| 自举 | `scripts/remote/bootstrap.sh` | .xsession 私总线/合成器/屏保固化 |

---

## 5. 已知陷阱登记表（持续追加）

| 编号 | 症状 | 根因 | 状态 |
|---|---|---|---|
| KI-004 | dev 二进制连不上 LAN | macOS TCC 拦未签名进程 | dev tunnel 缓解 |
| KI-013 | 桌面全黑但已连接 | RDPGFX 通道 handler 未接线 | 已修复 |
| KI-015 | 新建会话 WM 段错误 | xorgxrdp + WM 交互 | 重连既有会话 |
| KI-016 | 单实例应用开窗到物理屏 | 会话共享系统 user bus | 私总线修复 |
| KI-017 | WM 活但拖/最大化全失效 | xfwm4 GL 合成器挂死 | 关合成器+replace |
| KI-018 | 拖不动/黏窗/点击失灵 | **指针 release 编码错误（2.1）** | 已修复 |
| KI-019 | 剪贴板不通 | **四坑（2.2）** | 已修复 |
| 新 | 新症状… | 待分析 | |

※ KI 详文见 `docs/KNOWN_ISSUES.md`；嵌入式设计见 `docs/EMBEDDED_RDP.md`。

---

## 6. 提交与发布纪律

- **本机是父仓库**（根 `/Users/chenzibo/data/project`，混含多个项目）：提交时只 `git add` 本项目路径，`git status` 里其他项目的删除/未跟踪一律不碰。
- **发布仓** `github.com/zibochen6/jetson-tools` 是干净的独立副本；发布时从项目目录 rsync（排除 `.claude/ .trellis/ target/ node_modules/ dist/ tmp.txt .DS_Store`）。
- 功能改动按「feat/fix/docs + scope」风格提交；连接链路改动引用本文档第 3 节回归结果。
- 发布前扫描硬编码凭据：`grep -rniE "password|secret|token|api_key" src src-tauri scripts`。

---

*文档维护：2026-09-02 初版，沉淀 KI-004/013/015/016/017/018/019 经验。*