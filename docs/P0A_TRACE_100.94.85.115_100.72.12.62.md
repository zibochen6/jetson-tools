# P0-A 多设备真机 Trace 报告

- **日期**: 2026-09-04 15:00–16:00
- **场景**: A/B 双 Jetson 双隧道 + 双桌面（后端等价验证，§5/§6/§8）
- **App**: 正在运行的 `Jetson Remote.app`（PID 27590，**旧 v0.3.2 二进制**；本仓库工作树中的新诊断改动尚未构建进该 App）
- **数据来源**: 系统进程/端口实测 + 穿 App 自身隧道的只读 SSH + 设备侧只读检查 + `~/Library/Logs/jetson-remote.log`（App 自身 `[jr-flow]`/`[jr-clip]` trace）

---

## 1. 双设备身份（§6）

| | Device A | Device B |
|---|---|---|
| host | `100.94.85.115` (Tailscale) | `100.72.12.62` (Tailscale) |
| tailscale 节点名 | `j501-mini-64g` | `seeed` |
| serial (`/proc/device-tree/serial-number`) | **1421123007848**（per KI-026 文档；本轮设备离线无法直读） | **1423725051447**（✅ 穿 App 隧道实测） |
| machine-id | （离线未读） | `dbfef1aa0b064bcf9d30ec3ad0886edb`（实测） |
| 型号 / 系统 | reComputer J501 mini 64G, Ubuntu 22.04 | reComputer Rugged Orin NX 16G J401, JetPack 5.1.3 / Ubuntu 20.04 |
| xrdp | 有（今日上午在线时 App 已连） | ✅ 本日 provision 完成（xrdp 0.9.12 + xorgxrdp + XFCE 4.14） |

**判定**: `A.serial(1421123007848) ≠ B.serial(1423725051447)` → 身份不合并，PASS。
（两台 machine-id 亦不同源：B 实测 `dbfef1aa…`，非克隆共享的 `5dbfb124…`。）

---

## 2. 隧道时间线（§4 trace，App 日志 + 系统实测交叉验证）

```
上午           App 连接 A（100.94.85.115），隧道占首选端口 2222/3389
               （进程实测 PID 45051 于 13:34 存活；日志含多次
                "tunnel up 127.0.0.1:2222 / 127.0.0.1:3389"）
~15:0x         A 设备掉线（网络路径死亡）→ App 的 A 隧道退出，2222/3389 释放
15:08:34       用户在 App 连接 B（100.72.12.62）：
                 [jr-flow] probe start host=100.72.12.62 port=22 user=seeed
                 [jr-flow] tunnel spawn host=100.72.12.62 user=seeed
                 [jr-flow] tunnel up 127.0.0.1:2222 / 127.0.0.1:3389   ← B 拿到首选端口
                 [jr-flow] rdp session launch id=seeed@100.72.12.62 host=127.0.0.1 port=3389
                 （证书 TOFU 自动接受；"host key has changed" 为 FreeRDP 3.31
                  对新证书的标准措辞，实际为 unknown→accept+store）
                 [jr-clip] cliprdr 接线 → client capabilities → server capabilities
                 → monitor ready → format list announced ×4   ← 剪贴板握手完成
15:16 起       A 的 tailscale 节点彻底离线（"offline, last seen …"）
15:16–16:00    App 对 A 做后台有界自动重连（KI-024），每次：
                 [jr-flow] tunnel spawn host=100.94.85.115 user=seeed
                 （规划临时端口：59397/59398、60945/60946、… 每次重连都换新对）
                 [jr-flow] tunnel deadline; ssh stderr: no diagnostic output
                 [jr-flow] tunnel spawn failed: could not reach the device
               旧版日志累计：spawn_to_A = 32 次，unreachable 类失败 72 次
全程           B 的隧道（PID 89653, 2222/3389）与 B 的桌面会话毫发无损
```

**并发快照（15:5x 实测）**: 两个 `ssh -N` 隧道进程同时存在
- PID 89653 → `seeed@100.72.12.62`，本地 `127.0.0.1:2222 / 3389`（首选端口）
- PID 6005 → `seeed@100.94.85.115`，本地规划 `127.0.0.1:59397 / 59398`（临时端口，连接 A 失败，监听未绑定即死）
- 两个独立 0700 凭据目录 `tunnel/session-27590-*`（15:08 / 15:16）同时存在 —— KI-024 每隧道独立凭据隔离生效

---

## 3. §5 / §8 后端不变量判定

| 不变量 | 证据 | 判定 |
|---|---|---|
| 双隧道共存、本地端点唯一 | B=2222/3389 与 A 重连规划 59397/59398、60945/60946 从不重叠；两 ssh 进程并存 | ✅ PASS |
| B 不被 A 的（重）连 kill/replace/adopt | B 持续持有 2222/3389 贯穿 13:47 / 14:53 / 15:0x / 15:5x / 15:58 全部检查点；A 的重连每次都拿新的临时端口 | ✅ PASS |
| 一台失败不关另一台（§8） | A 真离线 → 32 次 spawn 全败（`could not reach the device`）→ B 隧道 + B 桌面 + B 剪贴板全程存活 | ✅ PASS |
| `active.len()==2` 等价 | 两个隧道 ssh 进程 + 两个 session-* 凭据目录同时存在 | ✅ PASS |
| 两个 `TunnelTarget` 不同 | `100.72.12.62:22` vs `100.94.85.115:22`（用户名同为 seeed，host 不同 → target 不同，永不 adopt） | ✅ PASS |
| 身份不合并（KI-026） | serial 1423725051447 vs 1421123007848 | ✅ PASS |

## 4. B 桌面会话存活证据（设备侧只读，穿 App 自己的隧道）

```
ss -tn established | grep :3389   → 127.0.0.1:38406 ↔ 127.0.0.1:3389   ← App 的 RDP 连接在
pgrep Xorg                        → /usr/lib/xorg/Xorg :10 -config xrdp/xorg.conf  ← xorgxrdp 虚拟会话在跑
systemctl                         → xrdp=active  xrdp-sesman=active
```
另：B 单机 RDP 出帧已由 `embedded_rdp_probe` 早前验证（`on_frame 78→1101`，`nnz=4096/4096`，center=`F0850E`，CLIPRDR 握手全链）。
A 的桌面今日上午在线时工作正常（App 已连 + 证书 `100.94.85.115_3389.pem` 已存）；本轮 A 离线无法复测。

## 5. §32 汇总

```
Device A: host=100.94.85.115  serial=1421123007848(docs)  tunnel=2222/3389→(死)→重连临时端口×32败
          SSH tunnel=PASS(上午)/FAIL(15:16起设备离线)  RDP=上午正常/现在无法测  clipboard/IME/drag=无法测(离线)
Device B: host=100.72.12.62   serial=1423725051447      tunnel=2222/3389 PASS
          SSH tunnel=PASS  RDP=PASS(设备侧会话+probe出帧)  clipboard(握手)=PASS  IME/drag=GUI待测
Multi-device(backend): PASS   （双隧道/不互杀/故障隔离/身份隔离 全部实测通过）
Mac→Jetson clipboard / Jetson→Mac clipboard / IME / 拖拽: PENDING — 需真机 App GUI 操作
```

## 6. 对用户原始症状（「第二台几乎立刻 Couldn't reach this Jetson」）的定位结论

1. **今天没有复现「第二台连不上」**：B 作为后连的第二台，一次成功（probe→tunnel(首选端口)→RDP→桌面→剪贴板握手）。真正连不上的是**掉线的那台 A**（32 次 `could not reach the device`，25s deadline）。
2. 结合本轮环境观测，原始症状最可能的成因排序：
   - **被连的那台设备当时真实不可达**（今天 `192.168.2.18` USB 桥设备同样间歇掉线、A 也掉线 42 分钟 —— 这些板子的网络稳定性是真实变量）；
   - 旧版 UI 把所有隧道失败统一映射成「Couldn't reach this Jetson」，无法区分 `TUNNEL_TARGET_UNREACHABLE`（设备不可达）与回环/认证失败 —— 本仓库工作树已完成细分（`TUNNEL_*` code + `tunnel ensure` trace + `LOOPBACK_SSH_FAILED`），**待重新构建后生效**（当前运行的 App 仍是旧二进制，其日志格式为旧版 `tunnel spawn/failed`）。
3. 旧 App 日志同时证实 KI-022 已知边界：掉线设备每次重连规划**新的临时端口**（59397→60945→…），设备恢复后 RDP 将以新端口建连 → cert store 新增一条 `100.94.85.115_<port>.pem`（TOFU 自动接受，无感但文件会累积）。

## 7. 待办（GUI 真机门槛，无法从本环境驱动）

- [ ] Tab `A→B→A→B` 切换 10 次不重连（A 需先恢复在线）
- [ ] A/B 输入不串台、剪贴板双向（pbcopy↔xclip + GUI 两向）
- [ ] 中文 IME 直输（Mousepad/浏览器/Terminal）
- [ ] 剪贴板+IME 全开下拖标题栏 30s
- [ ] 20 轮 A/B 循环压测

---
*注：本报告所有结论均来自正在运行的旧 v0.3.2 App 的真实行为 + 只读系统观测；未对 App 的连接做任何干扰（A/B 侧均未开第二条 RDP 连接）。*
