# 08 · 已知问题与实现状态

> 本文只记录**从代码可直接验证**的问题，不含产品规划。按影响面排序。

---

## 1. 高优先级缺陷

### P0-1 · QUIC 客户端不信任自签名证书

**位置**：[connection.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/network/connection.rs) → `create_client_config()`

```rust
let mut roots = rustls::RootCertStore::empty();       // 空信任库
let client_config = rustls::ClientConfig::builder()
    .with_safe_defaults()
    .with_root_certificates(roots)                    // 仍走默认 WebPKI 校验
    .with_no_client_auth();
```

服务端证书由 `rcgen` 现场生成（[connection.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/network/connection.rs) → `generate_self_signed_cert`），**没有任何 CA 签发**。rustls 默认校验器会以 `UnknownIssuer` 拒绝该证书，导致所有 `connect()` 失败。

注释写着 “permissive client config that accepts self-signed certificates”，但**未注册自定义 `ServerCertVerifier`**，与实际行为不符。

**影响**：设备发现（UDP）正常，但任何 QUIC 连接都会失败 → 传输不可用。

**修复方向**：

```rust
#[derive(Debug)]
struct SkipVerify;
impl rustls::client::ServerCertVerifier for SkipVerify { /* 全部 Ok */ }

let mut cfg = rustls::ClientConfig::builder()
    .with_safe_defaults()
    .with_custom_certificate_verifier(Arc::new(SkipVerify))
    .with_no_client_auth();
```

更好的做法：在发现报文里携带证书指纹，客户端按指纹固定（certificate pinning），而不是无条件跳过。

---

### P0-2 · 数据流与服务端认证握手不一致

**位置**：

- 客户端发送数据：[manager.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/transfer/manager.rs) → `send_files_data()`
- 服务端接入：[quic_server.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/network/quic_server.rs) → `handle_incoming_connection()`

```text
客户端 send_files_data：
    Endpoint::client(...) → connect(peer) → open_bi()
    → 直接写 FileHeader + DataPacket...          ← 无认证

服务端 handle_incoming_connection：
    accept_bi() (10s) → authenticate_as_server()  ← 期望首流是 AuthHello
```

两侧对「新连接的第一条流」的定义完全不同，服务端会把 `FileHeader` 当 `AuthHello` 反序列化 → 失败并关闭连接。同时客户端也**没有在这条连接上发送 `TransferRequest`**，而服务端是在请求流上直接 `receive_files(recv)` 的。

**影响**：即使修复 P0-1，数据仍然传不过去。

**修复方向**（择一）：

1. **单连接单流**：客户端在请求流上依次发送 `TransferRequest` → 读响应 → 继续在同一流发送文件数据（服务端已经是这个模型）；删掉 `send_files_data` 里新建连接的逻辑。
2. **双连接**：客户端在数据连接上先做认证 + 重发 `TransferRequest`；服务端为「数据连接」提供独立入口。

---

### P0-3 · 服务端 `receive_window` 只有 16 字节

**位置**：[connection.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/network/connection.rs) → `create_server_config()`

```rust
transport.receive_window(16u32.into())   // 16 B
```

QUIC 流控窗口为 16 字节，而每帧至少几十字节（Uuid 就 16 字节），连接启动后几乎立刻被窗口阻塞。

**修复**：改为 `(16 * 1024 * 1024u32).into()`（16 MB），并同步考虑 `stream_receive_window`。

---

### P0-4 · 连接池里的连接随 endpoint 一起被释放

**位置**：[quic_client.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/network/quic_client.rs) → `request_transfer()`

```rust
let client = QuicClient::new().await?;          // 新 endpoint
let response = client.send_transfer_request(...).await?;
// 函数结束，client 被 drop → endpoint 关闭 → 池中连接失效
```

quinn 的 `Endpoint` 必须保持存活才能维持连接。`QUIC_CLIENT` 单例保存的是 `Option<QuicClient>`，但 `get_quic_client()` 每次都返回**新实例**，旧 endpoint 无人持有。

**影响**：连接复用逻辑名存实亡，保活任务建立的连接也会被回收。

**修复方向**：`QuicClient` 用 `OnceCell<Arc<QuicClient>>` 长期持有；`request_transfer` 使用全局实例。

---

## 2. 中优先级问题

### 2.1 前后端参数不匹配

| 位置 | 前端发送 | 后端形参 | 结果 |
|------|----------|----------|------|
| `resend_files` | `{ deviceId: string, filePaths: string[] }` | `device_ids: Vec<String>, file_paths: Vec<String>` | 反序列化失败（当前前端未调用，暂未暴露） |

**修复**：统一为 `{ deviceIds: string[], filePaths: string[] }`。

### 2.2 `transfer-failed` 事件字段不一致

后端 [ipc/mod.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/ipc/mod.rs)：

```rust
json!({ "task_id": task_id.to_string(), "error": error })
```

前端 [tauriApi.ts](file:///D:/Projects/chuanshu-app/src/services/tauriApi.ts) 回调读取 `payload.taskId` → 永远 `undefined` → `setTaskStatus(undefined, 'Failed')` 不生效，UI 不会立即变红（靠下一次 2s 轮询兜底）。

**修复**：后端改为 `"taskId"`，或前端读 `payload.task_id`。

### 2.3 `DeviceInfo.last_seen` 类型不一致

后端 `SystemTime` 经 serde 序列化为：

```json
{ "secs_since_epoch": 1762300000, "nanos_since_epoch": 0 }
```

前端声明为 `last_seen: number`，`useDeviceStore` 排序与时间展示会对对象做算术 → `NaN`/`Invalid Date`。

**修复**：`#[serde(with = "...")]` 序列化为秒，或前端按对象取值。

### 2.4 设置的设备名从不生效

[lib.rs](file:///D:/Projects/chuanshu-app/src-tauri/src/lib.rs)：

```rust
let settings = storage.load_settings()?;
if let Some(settings) = settings {
    let _device_name = settings.device_name;   // 未使用
}
```

`AppState` 构造时写死 `"我的设备"`，保存设置也不会更新。`save_settings` 只写库。

**修复**：把 `AppState.device_name` 改为 `RwLock<String>`，保存时更新，或启动时传给 `AppState::new`。

### 2.5 限速单位不一致

- 前端滑块 0–100，标签 `MB/s`，直接存 `upload_limit`/`download_limit`。
- 后端 `RateLimiter::new(limit)` 按 **bytes/s** 处理。

选 10 → 实际 10 B/s，传输几乎停滞。

**修复**：前端 `× 1024 × 1024` 后再提交，或后端按 MB/s 解释。

### 2.6 断点续传未实现

- 发送端**从不保存** `ResumeInfo`（`ResumeManager::save_resume` 无调用方）。
- `send_files_data_with_resume(..., _resume_info, ...)` 参数带下划线，完全未使用。
- 接收端不支持“从偏移继续”，协议里也没有 `Resume` 请求类型。
- `resume_transfer` 只能把本地状态从 `Paused` 改回 `Transferring`。

**结论**：`resume.rs` 是完整的数据结构 + 部分 DAO，但没有接入任一传输分支。

### 2.7 认证只做了“打招呼”

- 客户端不校验 `AuthResponse.signature` / `cert_fingerprint`。
- 服务端不校验 `AuthAck.signature`。
- `cert_fingerprint` 是 `"mock-fingerprint-{uuid}"`，与 TLS 证书无关。

**影响**：局域网内任何能连上 45679 的主机都能冒充设备。分片 CRC32 只能防传输损坏，不能防篡改。

### 2.8 接收端可能“0 字节即完成”

`receive_files` 在 `Ok(())` 后无条件 `set_status(Completed)`。若流在首个 header 前就结束（`FinishedEarly`）会直接 `break`，任务仍被标记完成并写入历史。

**修复**：校验已接收字节数是否等于 `total_size`，不等则标记失败。

### 2.9 临时文件与目标路径安全

- 临时文件名 `.{file_id}.tmp` 不含 `task_id` → 并发接收多个任务时（每任务 file_id 从 0 开始）会互相覆盖。
- `download_path.join(relative_path).join(file_name)` 未做 `..` 过滤，恶意对端可路径穿越到下载目录之外。
- 同名文件直接 `rename` 覆盖，无去重。

### 2.10 保存路径与设置脱节

服务端在 `TransferResponse.save_path` 中**硬编码** `"~/Downloads/传书"`，与用户设置的 `download_path` 无关（实际落盘用的是设置值）。

### 2.11 进度事件重复

- 传输循环每 100ms 发一次。
- `start_progress_updater` 每 500ms 再把所有 `Transferring` 任务发一遍。

前端每秒最多处理约 12 次事件，属可优化项。

---

## 3. 技术债与改进点

### 3.1 未使用 / 半成品代码

| 项 | 位置 | 说明 |
|----|------|------|
| `MAX_CONCURRENT_TRANSFERS` | `core/constants.rs` | 无并发控制 |
| `cleanup_old_tasks` | `transfer/manager.rs` | 无调用方；内部用 `blocking_lock`（async 中会 panic） |
| `TransferManager.active_transfers` | `transfer/manager.rs` | 只 `retain`，从不 `push`；且每次命令都新建 manager，字段恒为空 |
| `BatchBuilder` / `batch_files` | `transfer/file_chunk.rs` | 未接入 |
| `FileChunker` / `FileAssembler` | `transfer/file_chunk.rs` | 实际传输用自己的循环 |
| `StreamHandle` / `StreamDirection` / `StreamStatus`、`PeerConnection.streams` | `network/connection.rs` | 未使用 |
| `ConnectionPool.pending` | `network/connection.rs` | 只读不写 |
| `capability::P2PTransfer` | `network/device.rs` | 未宣告、未使用 |
| `TransferTask.save_path` / `source_paths` | `transfer/task.rs` | 未写入 |
| `TransferStatus::Paused` 对端同步 | — | 只改本地 |
| `core/config.rs` | `core/` | 空文件 |
| 前端 `getDeviceList` / `deleteKnownDevice` / `openFileLocation` / `resendFiles` | `services/tauriApi.ts` | 无调用方 |
| 前端 `setDeviceStatus` / `addTask` / `updateTask` / `removeTask` | `store/*` | 无调用方 |
| `emit_device_online` / `emit_transfer_progress` | `ipc/mod.rs` | 定义了但均由其他模块直接 `emit` |
| `ResumeManager::save_resume` / `list_incomplete_transfers` / `cleanup_old_entries` | `transfer/resume.rs` | 无调用方 |
| `xxhash-rust` / `anyhow` 等依赖 | `Cargo.toml` | 未见使用 |

### 3.2 架构层

1. **模块循环依赖**：`ipc ↔ transfer`、`ipc ↔ storage`、`storage → ipc::Settings`。建议把 DTO（`Settings`、`SendFilesRequest`）下沉到 `core/types.rs`。
2. **全局 `OnceCell` 单例**：`DISCOVERY_SERVICE`、`QUIC_SERVER`、`CONNECTION_POOL`、`QUIC_CLIENT`、`STORAGE` 都是进程级，无法测试、无法重启服务（关闭后不能再次启动）。
3. **两套事件常量**：`ipc::DEVICE_*` 与实际 `app_handle.emit("device-online")` 的字符串字面量重复，容易漂移。
4. **同步 SQLite 阻塞 async worker**：高频写 `known_devices`（每 3s × N 设备）会占用 Tokio 线程。建议改用 `tokio::task::spawn_blocking` 或连接池 + `r2d2`。
5. **命令层直接 `Arc<AppState>` 参数**：Tauri 状态应以 `State<'_, Arc<AppState>>` 注入，当前多处 `app.state::<Arc<AppState>>().inner().clone()`。

### 3.3 前端层

1. **轮询 + 事件双通道**（2s / 5s），可只保留事件，或延长轮询周期。
2. **选择态不一致**：`useDeviceStore.selectedDeviceId` 存在但 `HomePage` 用自己的 `useState`。
3. **主题两份存储**：SQLite + `localStorage`（后者写入但从不读取）。
4. **Tailwind 4 配 v3 语法**：`@tailwind base;` 在 v4 下不生效；要么改 `@import "tailwindcss";` + `@config`，要么删除相关依赖。
5. **`react` / `react-dom` 放在 devDependencies**：不符合常规（虽不影响 Vite 打包）。
6. **无 lint/test**：没有 ESLint、Prettier、Vitest，纯靠 `tsc`。
7. **硬编码中文**：无 i18n；`SettingsPage` 的“快捷键”提示是假实现。
8. **历史页缺字段**：后端不返回 `save_path`，导致“打开位置”“重新发送”无法实现。
9. **`DeviceList` 头部统计文案**：统计全部已知设备（含离线），却写“N 个设备在线”。

### 3.4 协议与兼容性

1. **bincode 无版本协商**：字段一变即不兼容；`protocol_version` 只做字符串全等比较，没有迁移策略。
2. **`FileHeader` 两侧各自定义**：字段顺序必须手工保持一致，建议抽到 `protocol.rs` 共用。
3. **未使用 `TransferRequest.resume`** 字段。
4. **控制消息类型 `QueryProgress` / `ProgressUpdate` / `Error` 无人处理**。
5. **`send_control` 用单向流，服务端只 `accept_bi`**：取消/暂停通知实际到不了对端。

### 3.5 构建与 CI

1. `build.yml` Ubuntu 依赖 `libgstreameller-plugins-bad-dev` 拼写错误（应为 `libgstreamer-plugins-bad1.0-dev` 或删除该行）。
2. `setup-node` 的 `cache: pnpm` 在 `pnpm/action-setup` 之前，缓存步骤可能失败。
3. 两个 workflow 触发条件重叠，重复构建。
4. `tauri.conf.json` 中 `csp: null`，生产环境建议配置最小 CSP。
5. `version` 在 `package.json`、`Cargo.toml`、`tauri.conf.json` 三处手工同步。
6. 未签名/未公证：macOS 用户首次打开需右键“打开”；Windows 会提示未知发布者。

---

## 4. 功能实现状态矩阵

以代码实际行为为准（✅ 可用 / 🟡 部分可用 / 🔴 未实现）：

| 功能 | 状态 | 说明 |
|------|------|------|
| UDP 设备广播发现 | ✅ | 3s 心跳、30s 超时、多网卡广播 |
| 已知设备 IP 探测 | ✅ | 每 10s 单播，跨子网可用 |
| 手动 IP 发现 | ✅ | 3s 超时 |
| 设备列表（在线/离线） | 🟡 | 后端只发出 online/offline，前端四态中的 scanning/connecting 无来源 |
| 修改设备名称 | 🔴 | 保存成功但不生效 |
| QUIC 建连 | 🔴 | 自签名证书被客户端拒绝（P0-1） |
| 发送文件（单/多设备） | 🔴 | 数据流与认证流不一致（P0-2）、流控窗口异常（P0-3） |
| 接收文件并落盘 | 🟡 | 落盘逻辑完整，但上游链路不通；无 0 字节校验 |
| 文件夹递归发送 | ✅ | `walkdir` 展开，保留相对路径 |
| 分片 + CRC32 校验 | ✅ | 4MB 分片 |
| 整文件 SHA-256 | 🟡 | 仅 <100MB 文件计算 |
| 进度显示 | ✅ | 100ms + 500ms 双通道 |
| 暂停 / 继续 / 取消 | 🟡 | 仅本地状态，不通知对端 |
| 断点续传 | 🔴 | 数据结构完整，未接入流程 |
| 传输历史 | ✅ | 记录终态，最多展示 100 条 |
| 清空历史 | ✅ | — |
| 打开文件位置 | 🟡 | 命令可用，历史页未接（无路径字段） |
| 重新发送 | 🔴 | 命令参数不匹配，页面为占位 |
| 设置读写 | 🟡 | 可读写；限速单位错误、设备名不生效 |
| 自动接收 | 🟡 | 后端已读设置，但传输链路不通 |
| 系统通知 | ✅ | 完成/失败通知，受开关控制 |
| 开机自启 | ✅ | 三平台实现 |
| 深色模式 | ✅ | 系统跟随 + 手动切换 |
| 传输并发上限 | 🔴 | 常量未使用 |

---

## 5. 建议的修复顺序

```mermaid
flowchart LR
    A[P0-1 证书校验] --> B[P0-3 流控窗口]
    B --> C[P0-2 数据流模型统一]
    C --> D[P0-4 endpoint 生命周期]
    D --> E[验证端到端传输]
    E --> F[2.2/2.3 事件与类型对齐]
    F --> G[2.4/2.5 设置生效]
    G --> H[2.6 续传真正实现]
    H --> I[3.x 技术债清理]
```

1. **先让链路通**：证书校验 → 流控窗口 → 统一「请求 + 数据」的流模型 → endpoint 生命周期。
2. **再让状态对**：`transfer-failed` 字段、`last_seen` 序列化、限速单位、设备名生效。
3. **最后补功能**：断点续传、历史页操作、并发控制、命名冲突与路径安全。
4. **长期**：下沉 DTO 消除循环依赖；把 `OnceCell` 换成可注入的服务容器；补集成测试（两台进程 loopback 互传）。

---

## 6. 安全提示（局域网场景）

| 风险 | 现状 | 建议 |
|------|------|------|
| 中间人 | 认证不校验签名、不固定证书 | 证书指纹随发现报文交换并在客户端比对 |
| 未授权接收 | `auto_accept` 默认 false（安全），但一旦为 true 任何人可发 | 增加配对码/白名单 |
| 路径穿越 | `relative_path` 未净化 | `canonicalize` 后校验前缀 |
| 文件名覆盖 | 直接 rename | 重名加序号 |
| CSP 关闭 | `csp: null` | 生产配置 `default-src 'self'` |
| 明文元数据 | UDP/QUIC 元数据未加密（QUIC 自带传输加密） | 业务层无需额外处理，但不要把敏感信息放进 `device_name` |
