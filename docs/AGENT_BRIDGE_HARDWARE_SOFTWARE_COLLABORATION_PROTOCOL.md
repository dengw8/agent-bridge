# Agent Bridge 软硬件协作与接口协议（前期沟通版）

> 状态：Draft v0.1  
> 日期：2026-08-05  
> 用途：用于与外部硬件开发方进行前期技术沟通、工作量评估和敏捷联调  
> 非用途：本文不是商务合同、知识产权协议或量产认证文件  
> 数据级别：内部；对外发送前按第 8 节执行最小披露

## 1. 目标

本协议用于明确 Agent Bridge App 与 Agent Bridge Device 之间的技术边界，使软件和硬件可以并行开发，并通过稳定的接口进行联调。

当前目标是完成 1–5 台自用设备。设备包含实体按键、摇杆、编码器、灯光、USB/BLE、电池和固件；macOS App 负责配置、动作含义解析以及与目标桌面应用的交互。

双方前期协作只需对齐以下内容：

1. 哪些逻辑由设备负责，哪些逻辑由 App 负责；
2. USB/BLE 如何承载统一的 ABP v1；
3. 输入、状态、断线、认证和升级如何交互；
4. 双方各自提供什么测试材料；
5. 接口变化如何快速提出、确认和回归；
6. 哪些敏感数据禁止进入硬件、固件和外部开发环境。

## 2. 系统边界

```text
物理按键 / 摇杆 / 编码器
            ↓
       设备端固件
            ↓
 USB Vendor HID / BLE GATT
            ↓
     Agent Bridge App
            ↓
  App 内部动作解析与执行

App 输出有限状态
            ↓
 USB Vendor HID / BLE GATT
            ↓
      设备端灯光渲染
```

### 2.1 设备端负责

- 扫描 13 个物理按键并完成消抖；
- 采集双轴模拟摇杆，完成校准、滤波、死区和方向状态机；
- 采集编码器旋转和按压；
- 处理 Profile Key、USB/BT1/BT2/BT3 模式及三个 BLE Bond Slot；
- 通过 USB Vendor HID 或 BLE GATT 收发 ABP v1；
- 维护设备启动代次、发送序号、输入快照、心跳和连接状态；
- 执行设备 Claim/Auth、固件升级、回滚和 Recovery；
- 接收 App 下发的完整语义状态并在本地渲染灯效；
- 管理电池、充电状态、电量估算、VLED、电源预算和低功耗；
- 提供有界、脱敏的设备状态和错误计数。

### 2.2 App 端负责

- 枚举并连接 USB/BLE 设备；
- 校验 ABP 帧、版本、方向、序号、启动代次和设备会话；
- 完成设备信任、活跃设备选择、断线清理和快照恢复；
- 将物理位置事件转换为 App 内部的规范化输入；
- 解析具体动作、Binding、Mic Combined/Split、Encoder Mode 和 Voice Mode；
- 控制目标桌面应用；
- 将 Agent、Mic、连接、电量等有限语义状态下发给设备；
- 在 macOS Keychain 中保存主机侧信任材料；
- 提供设置、校准、诊断、升级和恢复入口。

### 2.3 明确禁止的越界

设备和固件不得：

- 发送普通 Keyboard/Mouse 扫描码；
- 保存或解析 App 的 Binding、命令名称、快捷键或业务动作；
- 访问或保存目标桌面应用账号、任务、会话或内容；
- 直接执行 Mic、Approve、Reject 等具体业务动作；
- 通过 Wi-Fi、云端或隐藏遥测上报数据；
- 在重连、重启或恢复后补发历史输入。

App 不直接依赖设备内部的 GPIO、扫描矩阵、ADC、灯珠顺序或电源实现。上述实现细节由设备固件封装在 ABP v1 接口之后。

## 3. 控件语义

设备只上报“物理位置发生了什么”，不上报“这个位置代表什么动作”。

| 控件类型 | Wire ID | 设备上报 |
| --- | --- | --- |
| Agent Key | `AG00`–`AG05` | `Pressed` / `Released` |
| Action Position | `ACT06`–`ACT12` | `Pressed` / `Released` |
| Joystick | `Up/Right/Down/Left` | `Entered` / `Centered` |
| Encoder Press | `0` | `Pressed` / `Released` |
| Encoder Rotate | `0` | `Rotated(-32..-1 / 1..32)` |

特别约定：

- ACT10 和 ACT11 在 Wire 层始终是两个独立位置；
- 默认双宽 Mic 是否合并，由 App 的 Combined/Split Resolver 决定；
- click、double-click、long-press 等手势不在设备端合成；
- 编码器的具体模式和动作由 App 决定；
- 摇杆方向改变时，设备先上报旧方向 `Centered`，再上报新方向 `Entered`。

## 4. 通信协议

### 4.1 唯一协议权威

字节级格式、字段、枚举、方向、合法范围、状态机和 Golden Packets 以以下文件为唯一实现依据：

- [`AGENT_BRIDGE_PROTOCOL_V1.md`](./AGENT_BRIDGE_PROTOCOL_V1.md)
- 当前版本：Frozen v1.0
- 当前 SHA-256：`8ff30cdd8faa2692505ebc77aae807e253925c105b62cc4171b93adac14051be`

本文只描述协作边界，不重复定义 Wire 字节。若本文与 ABP v1 字节级定义冲突，以 ABP v1 为准。

### 4.2 传输约定

USB：

- 使用 Vendor-defined HID；
- 不暴露标准 Keyboard、Mouse、CDC 或 Mass Storage；
- HID Report 固定为 64 字节：1 字节 Report ID + 63 字节 ABP Frame；
- Device → Host 使用 Input Report ID `0x01`；
- Host → Device 使用 Output Report ID `0x02`。

BLE：

- 使用自定义 GATT Service；
- Host → Device 使用 Write/Write Without Response；
- Device → Host 使用 Notify；
- USB 和 BLE 承载完全相同的 63 字节 ABP Frame；
- MTU 不足时只在 BLE Adapter 层分片，上层 ABP 不感知；
- 分片乱序、重复、缺失、不一致或超过 100ms 时丢弃整个重组缓冲区。

### 4.3 协议基本不变量

- 所有输入必须来自完整、合法、方向正确且版本兼容的帧；
- 未知枚举、未知消息、错误长度、非零保留位和非规范填充全部拒绝；
- 设备和 App 各自维护独立的单调 `u32 sequence`；
- 重复帧和倒序帧丢弃，不产生状态变化；
- 序号缺口立即清理该来源的保持状态并请求完整输入快照；
- 每次设备启动生成新的非零 `boot_id`；
- `boot_id` 变化必须重新执行 Hello 和 Snapshot；
- App 为每次连接分配仅存在于主机侧的 Connection Generation；设备不得在 Wire 中指定该值，旧 Generation 不得影响当前会话；
- Ready 之前不执行普通输入；
- 首次进入 Ready 时，Snapshot 中已经按住或进入的控件只抑制到释放，不补发历史 Pressed/Entered；
- 连接中断只清理该连接来源的保持状态，不影响其他输入来源；
- 重连、重启和快照恢复不得合成或重放 Pressed 事件。

## 5. 会话交互

### 5.1 H0 最小联调

```text
USB Connected
  Device → Host: DEVICE_HELLO
  Host → Device: GET_INPUT_SNAPSHOT
  Device → Host: INPUT_SNAPSHOT
  Session: Ready
```

H0 用于证明真实硬件、USB、协议和主机 App 的最小纵向链路。H0 不要求 BLE、三槽位、完整认证或电池功能。

H0 联调至少覆盖：

- 一个 Agent Key；
- 一个 Action Position；
- 摇杆四方向和回中；
- 编码器正反转及按压；
- App → Device 的代表性灯光状态；
- USB 拔插、设备重启、序号缺口和按住时断线；
- 非法、截断、未知版本和畸形帧 fail closed；
- 端到端 USB 输入延迟 p95 小于 100ms。

### 5.2 H1+ 完整会话

```text
Transport Connected
  Device → Host: DEVICE_HELLO
  Host → Device: HOST_HELLO
  Host → Device: AUTH_CHALLENGE
  Device → Host: AUTH_RESPONSE
  Host → Device: GET_INPUT_SNAPSHOT
  Device → Host: INPUT_SNAPSHOT
  Host → Device: GET_DEVICE_STATUS
  Device → Host: DEVICE_STATUS
  Host → Device: PING / VISUAL_STATE
  Device → Host: PONG
  Session: Ready
```

H1+ 增加：

- USB、BT1、BT2、BT3 四种显式连接模式；
- 三个 BLE Host/Bond Slot；
- 首次 Claim 的实体 Profile Key 确认；
- 后续 HMAC challenge-response；
- BLE LE Secure Connections 与 Bonding；
- 设备状态、电池状态、心跳和离线状态；
- 固件升级、验证、回滚和 Recovery。

认证失败、版本不兼容或快照未完成时，设备不得产生可执行输入，并应显示明确的非 Ready 状态。

## 6. App 下发状态

App 只下发封闭枚举和数值，不下发内容或任意字符串。设备根据语义状态本地渲染颜色和动画。

硬件输出的字段解释、参考灯效、渲染优先级、自动熄灯和可直接用于联调的示例帧见
[`AGENT_BRIDGE_HARDWARE_OUTPUT_PROTOCOL.md`](./AGENT_BRIDGE_HARDWARE_OUTPUT_PROTOCOL.md)。该文件不改变 ABP v1 的字节格式，只将 `VISUAL_STATE` 和 `SET_DEVICE_CONFIG` 具体落实为硬件输出约定。

主要状态包括：

- 6 个 Agent Slot：Empty、Idle、Thinking、Complete、Requires Input、Error；
- 当前选中的 Agent Slot；
- Mic：Idle、Recording、Processing、Ready、Locked、Stopping、Unknown；
- Bridge：Offline、Ready、Degraded、Incompatible；
- Device Connection：Offline、Ready、Pairing、Claiming、Updating；
- 当前 Profile、近似电量、充电/外部电源/低电量标志；
- 亮度和 Auto-dim。

状态下发规则：

- App 发送完整状态，不依赖设备重放历史增量；
- 设备可在本地执行动画，但不得改变状态含义；
- 设备离线时不得把最后一次状态继续表现为“仍可执行”；
- App 状态变化到设备可见反馈的目标时间不超过 1 秒；
- 未知或不兼容状态必须保守处理，不得猜测。

## 7. 双方交付的联调材料

### 7.1 软件方提供给硬件方

前期默认只提供以下最小资料，不提供完整软件仓库：

1. `AGENT_BRIDGE_PROTOCOL_V1.md` 的受控版本；
2. Golden Packet manifest、合法二进制帧和非法语料；
3. 设备端可移植的 ABP 编解码测试约定；
4. H0 USB HID 描述符及开发 VID/PID/Usage 配置；
5. Fake Host 或最小联调工具；
6. 状态回放工具或固定 VisualState 测试序列；
7. 脱敏的问题日志格式和问题单模板；
8. 必要的硬件技术设计节选。

### 7.2 硬件方提供给软件方

1. ESP-IDF 固件源码及可重复构建说明；
2. ABP v1 C 端单元测试和 Golden Packet 结果；
3. USB HID 描述符、BLE Service/Characteristic 定义和固件版本；
4. 设备状态机、输入扫描和断线恢复说明；
5. 每次可联调固件的硬件修订、固件版本和构建标识；
6. 可复现问题的最短步骤、设备日志和必要的脱敏原始帧；
7. H0/H1 真实设备或远程可观察的联调环境；
8. 已知限制、未完成消息和协议偏差清单。

### 7.3 联调一致性要求

- Rust 与 C 对同一 Golden Packet 必须解码为相同字段；
- 双方重新编码后的合法帧必须逐字节一致；
- 双方对非法帧必须拒绝，且拒绝不得改变输入或会话状态；
- 每个 ABP v1 消息至少有一个合法样例和一个代表性非法样例；
- 测试失败时优先共享最小帧、状态和版本，不共享真实业务上下文。

## 8. 敏感数据边界

### 8.1 硬件方不需要获得

- 目标桌面应用、模型服务或其他真实账号；
- Cookie、Token、API Key、macOS Keychain 内容；
- 真实任务标题、会话 ID、Prompt、回复、终端内容、代码差异或音频；
- 生产固件签名私钥；
- 与硬件接口无关的主机软件源代码；
- 真实用户个人信息。

### 8.2 联调数据规则

- 默认使用 Fake Host、Golden Packets、虚构状态和测试密钥；
- 生产私钥由软件方/项目方离线保管，硬件方只使用开发测试密钥；
- 真实目标桌面应用副作用由软件方在受控环境执行；
- 日志只记录协议版本、消息类型、序号、状态枚举、错误码、时间和脱敏设备标识；
- 日志不得记录任意业务字符串或原始凭据；
- 对外发送日志、截图、视频和样机照片前，双方先检查账号、路径、序列号、通知内容和背景信息；
- 未经项目方明确同意，不将源码、协议、日志或设计输入公共仓库、公共网盘、生成式 AI、在线 PCB/CAD Viewer 或其他会留存数据的第三方平台；
- PCB/SMT/3D 打印外发时只提供制造所需的最小文件，不同时提供软件源码、完整协议或密钥。

### 8.3 安全事件

发现凭据、密钥、源码或非预期业务数据进入设备、日志或外部平台时：

1. 立即停止同步、上传和相关联调；
2. 保留必要证据但不继续扩散；
3. 第一时间通知对方；
4. 明确影响范围并轮换测试/生产凭据；
5. 修复数据路径并增加回归测试后再恢复联调。

## 9. 敏捷协作方式

### 9.1 沟通节奏

- 启动会：一次，确认范围、基线、硬件方案、人员和联调环境；
- 周同步：建议每周 30 分钟，只讨论完成项、风险、下一步和待决策项；
- 阶段联调：H0、H1、Rev A、Rev B 各至少一次；
- 阻断问题：发现后当天同步，不等待周会；
- 日常讨论可以使用双方约定的即时通信工具，但接口结论必须回写到文档或 Issue。

### 9.2 单个问题的最小信息

每个软硬件交互问题至少包含：

```text
标题：一句话描述现象
硬件修订：H0/H1/Rev A/Rev B + revision
固件版本：版本号或 commit
App 版本：版本号或 commit
传输：USB / BT1 / BT2 / BT3
前置状态：boot_id、session state、是否 Ready（均须脱敏）
复现步骤：最短步骤
预期结果：
实际结果：
最小证据：消息类型/sequence/错误码/必要的脱敏帧
安全影响：无 / 输入副作用 / 数据 / 密钥 / 电池电源
```

### 9.3 决策时限建议

- 普通接口疑问：双方尽量在 1 个工作日内确认；
- 阻断 H0/H1 的接口问题：当天建立问题单并指定单一负责人；
- 涉及 Wire、认证、升级、电源或数据边界的变化：先停在现有基线，完成书面评审后再实现；
- 主观外观和手感问题不得阻塞不相关的协议联调。

## 10. 接口变更流程

ABP v1 当前为 Frozen v1.0。前期开发仍允许提出问题，但不得各自静默修改。

### 10.1 变更提案必须包含

- 当前问题和可复现证据；
- 为什么不能在现有实现内部解决；
- 建议修改的消息、字段、枚举或状态机；
- 是否破坏 v1.0 兼容；
- Rust、C、USB、BLE、测试和已制硬件的影响；
- 数据安全和重放/断线风险；
- 新增 Golden Packet 和非法语料；
- 回退方案。

### 10.2 版本规则

- 修正文档歧义但不改变 Wire：不升级 Wire 版本，记录澄清；
- 向后兼容新增：提升 minor，并保留 v1.0 Canonical Packet；
- 字段、枚举、安全或状态机的不兼容变化：提升 major；
- 高版本 minor 在未完成兼容评审前不得被低版本实现猜测接受；
- 任一变更只有在协议文档、双方实现和 Golden Packets 同时更新后才算完成。

## 11. 前期联调完成标准

前期可以按以下顺序逐步关闭风险，不要求一次交付完整产品：

### Checkpoint A：离线协议互测

- 双方可解析和重新编码全部 Golden Packets；
- 非法帧均 fail closed；
- Rust/C 输出逐字节一致；
- 不使用真实账号或业务数据。

### Checkpoint B：真实 USB H0

- 真实 ESP32-S3 Vendor HID 可被 macOS App 精确枚举；
- 代表性按键、摇杆、编码器输入可到达 App；
- App 的代表性 VisualState 可到达设备；
- p95 输入延迟小于 100ms；
- 拔插、重启、序号缺口和按住断线均不重放、不卡键。

### Checkpoint C：BLE 与完整会话

- USB 与 BLE 传输相同 ABP v1 语义；
- BLE 分片边界、超时和错误路径通过；
- 三个 Host/Bond Slot 可切换和重连；
- Claim/Auth、Snapshot、Heartbeat 和 Offline 行为通过；
- 非 Ready 状态不产生可执行输入。

### Checkpoint D：固件升级与恢复

- App 可通过 USB 传输升级镜像；
- 错误签名、错误硬件修订和降级均被拒绝；
- 中断升级可回滚；
- Recovery 可在真实板上恢复签名固件；
- 生产签名私钥始终不进入硬件方日常开发环境。

## 12. 前期需要硬件开发方确认的问题

硬件开发方在评估后请书面确认：

1. 是否接受“设备只上报物理语义、动作含义完全由 App 负责”的边界；
2. ESP32-S3 + ESP-IDF 是否为可接受的固件平台；
3. 是否能够实现 USB Vendor HID 和 BLE 自定义 GATT 的相同 ABP v1 语义；
4. 是否能够用 C 端测试消费现有 Golden Packets，并与 Rust 端逐字节互测；
5. 是否有多主机 BLE、应用层 Claim/Auth、断线快照恢复的实施经验；
6. 是否能够提供真实 H0/H1 台架和必要的 USB/BLE/电源测量证据；
7. 是否接受不接触真实账号、真实会话和生产签名私钥的联调方式；
8. 对 ABP v1、硬件选型、工期和主要风险有哪些需要在启动会前澄清的问题；
9. 联调负责人、日常沟通渠道和可用于联调的最早时间；
10. 需要软件方提前提供的最小测试材料清单。

## 13. 关联技术资料

前期沟通按最小必要原则提供，下列资料不必一次性全部外发：

1. [`AGENT_BRIDGE_PROTOCOL_V1.md`](./AGENT_BRIDGE_PROTOCOL_V1.md)：Wire 协议唯一权威，优先提供；
2. Golden Packets/invalid corpus：离线互测所需，优先提供；
3. [`AGENT_BRIDGE_HARDWARE_STAGE_TARGETS.md`](./AGENT_BRIDGE_HARDWARE_STAGE_TARGETS.md)：需要评估阶段工作量时提供；
4. [`AGENT_BRIDGE_HARDWARE_TECHNICAL_DESIGN.md`](./AGENT_BRIDGE_HARDWARE_TECHNICAL_DESIGN.md)：在确认合作意向并建立受控共享渠道后，按章节提供；
5. App 侧 Fake Host、H0 联调工具和脱敏诊断格式：进入实际联调时提供；
6. 完整主机软件仓库、真实目标桌面应用环境和生产密钥：默认不提供。

本协议的目标是尽早形成一个真实、可测、无敏感数据的纵向切片，再逐步扩展到 BLE、电源、完整设备和生产包。
