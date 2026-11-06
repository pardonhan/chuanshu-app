# 07 · 协议与数据流

## 1. 线格式总览

| 通道 | 传输 | 帧格式 | 序列化 |
| --- | --- | --- | --- |
| 设备发现 | UDP `45678` | 单个 `DiscoveryPacket`（无长度前缀） | bincode |
| 认证 | 每条 QUIC 连接的第一条双向流 | `u32` 大端长度 + `ControlMessage` | bincode（嵌套） |
| 控制消息 | 每条新建双向流一条消息 | 同上 | bincode |
| 数据传输 | 一条双向流内连续多帧 | `u32` 大端长度 + `FileHeader` / `DataPacket`；长度 0 表示结束 | bincode |

`ControlMessage` 的结构是：

```rust
pub struct ControlMessage {
    pub protocol_version: String,
    pub message_type: ControlMessageType,
    pub payload: Vec<u8>,   // 具体结构再 bincode 一次
}
```

即「长度前缀 → bincode(ControlMessage) → payload 内部再 bincode」。接收端限制单条控制消息 ≤ **10 MiB**，并校验 `protocol_version == 1.1.0`。

```mermaid
flowchart LR
  A["4 字节大端长度 L"] --> B["L 字节 ControlMessage"]
  B --> C["message_type"]
  B --> D["payload: Vec<u8>"]
  D --> E["bincode 反序列化为具体类型"]
```

## 2. 消息类型表

### 2.1 发现层

| `DiscoveryMessageType` | 方向 | 触发 | 载荷结构 |
| --- | --- | --- | --- |
| `Announce` | 广播 / 定向 | 每 3s；已知设备探测每 10s；手动 IP 发现 | `DiscoveryPacket`（含证书指纹） |
| `Response` | 单播回源 | 收到 `Announce` 后 | `DiscoveryPacket`（`ip_address=None`，接收方用源地址） |
| `Goodbye` | 广播 | 应用退出、停止发现服务 | `DiscoveryPacket`（能力为空） |

### 2.2 控制层

| `ControlMessageType` | 方向 | payload | 说明 |
| --- | --- | --- | --- |
| `AuthHello` | C → S | `AuthHello` | 第一条流，携带客户端证书指纹 |
| `AuthResponse` | S → C | `AuthResponse` | 含本机证书指纹与 nonce 签名 |
| `AuthAck` | C → S | `AuthAck` | 含对服务端 nonce 的签名 |
| `TransferRequest` | C → S | `TransferRequest` | 文件清单 + 总大小 + `resume` |
| `TransferResponse` | S → C | `TransferResponse` | 接受/拒绝 + 建议保存路径 |
| `DataTransferStart` | C → S | `DataTransferStart` | 开一条新流，声明本次数据流属于哪个任务 |
| `DataTransferReady` | S → C | `DataTransferReady` | 接受 + 每文件缺失分片 |
| `CancelTransfer` | 双向 | `Uuid` | 任务 ID |
| `PauseTransfer` | 双向 | `Uuid` | 任务 ID |
| `ResumeTransfer` | 双向 | `Uuid` | 任务 ID |

> `QueryProgress` / `ProgressUpdate` / `Error` 等未实现枚举已从协议中移除。

### 2.3 数据结构字段

`FileMetadata`：

| 字段 | 说明 |
| --- | --- |
| `file_id: u64` | 任务内唯一，按收集顺序递增 |
| `relative_path: String` | 目录传输时保留顶层文件夹（如 `src/utils`） |
| `file_name: String` | 文件名 |
| `file_size: u64` | 字节 |
| `modified_time: u64` | Unix 秒 |
| `checksum: Option<String>` | <100MB 文件的 SHA-256 |
| `is_directory: bool` | 恒为 `false` |
| `source_path: String` | `#[serde(skip)]`，不传输 |

`TransferRequest`：`task_id, sender_id, sender_name, file_count, total_size, files, resume`。

`TransferResponse`：`task_id, accepted, reason, save_path`。

`DataTransferStart`：`task_id, files, resume`。

`DataTransferReady`：`task_id, accepted, reason, missing_chunks: HashMap<file_id, Vec<chunk_index>>`。

`ChunkMetadata`：`task_id, file_id, chunk_index, total_chunks, chunk_size, offset, checksum(CRC32)`。

`DataPacket`：`metadata: ChunkMetadata` + `data: Vec<u8>`。

## 3. 设备发现时序

### 3.1 同网段自动发现

```mermaid
sequenceDiagram
  participant A as 设备A（发现服务）
  participant Net as 局域网
  participant B as 设备B（发现服务）

  loop 每 3 秒
    A->>Net: Announce 广播 (45678)
    Net->>B: Announce
    B->>B: 校验 device_id ≠ 本机、version == 1.1.0
    B->>B: 写入 devices + known_devices
    B->>B: 新设备 → emit device-online
    B->>A: Response 单播
    A->>A: 写入 devices + known_devices
    A->>A: 新设备/曾离线 → emit device-online
    A->>A: spawn QUIC 保活连接
  end

  Note over B: 30 秒未再收到 A 的心跳<br/>清理任务移除 A 并 emit device-offline
```

### 3.2 跨子网 / 手动发现

```mermaid
sequenceDiagram
  participant A as 设备A
  participant DB as known_devices
  participant B as 设备B

  loop 每 10 秒
    A->>DB: get_known_devices()
    DB-->>A: 设备列表
    A->>B: Announce 单播到 ip:45678
    B->>A: Response
    A->>A: handle_discovery_packet → 更新 + 可能建立 QUIC
  end

  Note over A: 手动 IP 发现：<br/>发送 Announce，3 秒内等 Response
```

### 3.3 离线判定

| 触发 | 处理 |
| --- | --- |
| 收到 `Goodbye` | 立即移除、置库离线、emit `device-offline` |
| 30s 清理任务 | `last_seen` 超过 30s → 移除、置离线、emit |
| QUIC 连接关闭回调 | 置库离线、emit `device-offline`（不从 `devices` 移除） |
| 下次 `Announce`/`Response` 且曾离线（>25s） | 重新建立 QUIC、emit `device-online` |

> 25s 与 30s 两个阈值并存：25s 用于“曾离线”判断（决定是否重连/重发事件），30s 用于实际移除。

## 4. 认证时序

```mermaid
sequenceDiagram
  participant C as 客户端
  participant S as 服务端

  C->>C: nonce_c = random(32)
  C->>C: 校验 TLS 证书指纹（PinnedCertVerifier）
  C->>S: AuthHello { device_id_c, name_c, version, cert_fingerprint_c, nonce_c }
  S->>S: nonce_s = random(32)
  S->>S: sig_s = SHA256(nonce_c ‖ device_id_s)
  S->>S: fingerprint = AppState.certificate_fingerprint()
  S->>C: AuthResponse { device_id_s, name_s, fingerprint, sig_s, nonce_s }
  C->>C: 校验 sig_s == SHA256(nonce_c ‖ device_id_s)
  C->>C: 校验 fingerprint 非空且与发现阶段记录一致
  C->>C: 写入 devices[device_id_s]
  C->>S: AuthAck { device_id_c, sig_c = SHA256(nonce_s ‖ device_id_c), authenticated=true }
  S->>S: 校验 authenticated、device_id_c、sig_c == SHA256(nonce_s ‖ device_id_c)
  S->>S: 写入 devices[device_id_c]
  S->>S: 连接入池并 mark_authenticated
```

认证只发生在**每条 QUIC 连接的第一条双向流**；客户端用 `ensure_authenticated` 保证只执行一次，服务端在 `handle_incoming_connection` 中同步完成。

## 5. 发送 → 接收完整时序

```mermaid
sequenceDiagram
  autonumber
  participant UI as 前端
  participant IPC as Tauri IPC
  participant M as TransferManager
  participant C as QuicClient / 连接池
  participant S as QuicServer
  participant RUI as 接收端前端
  participant FS as 接收端磁盘

  UI->>IPC: sendFiles(deviceIds, filePaths)
  IPC->>M: send_files(request)
  M->>M: collect_files → FileMetadata[]
  M->>M: 为每个设备创建 TransferTask(Pending)
  M-->>IPC: task_ids
  M->>M: spawn initiate_transfer
  M->>C: request_transfer(addr, peer_id, TransferRequest)
  C->>C: get_or_connect + ensure_authenticated
  C->>S: 新流：TransferRequest
  S->>S: 校验 sender_id == 已认证 peer
  alt auto_accept = false
    S->>RUI: emit transfer-request
    RUI->>S: accept_transfer_request(task_id) → oneshot(true)
  else auto_accept = true
    S->>S: accepted = true
  end
  S->>S: 创建 Receive 任务（files 暂空）
  S->>C: TransferResponse { accepted, save_path }
  C-->>M: response + PeerConnection
  M->>M: 任务置 Transferring
  M->>C: send_files_data（新流）
  C->>S: DataTransferStart { task_id, files, resume }
  S->>S: 校验任务归属，写 files，状态 Transferring
  S->>S: 读断点 → 计算 missing_chunks
  S->>C: DataTransferReady { missing_chunks }
  loop 每个文件
    C->>S: FileHeader（长度前缀）
    loop 需要的分片
      C->>S: DataPacket（长度前缀）
      S->>S: CRC32 校验 + seek + write
      S->>S: 每 4 块保存断点
      S->>RUI: transfer-progress（≥100ms）
    end
    S->>S: flush/sync + SHA-256（若有）
    S->>FS: rename 临时文件到最终路径
  end
  C->>S: 0u32 结束标记 + finish
  S->>S: 删除 resume_info、写历史、置 Completed
  C->>C: 等 ack（最多 5 秒，通常因流关闭提前返回）
  M->>M: 置 Completed、写历史、emit transfer-completed、通知
```

## 6. 暂停 / 恢复 / 取消时序

### 6.1 暂停

```mermaid
sequenceDiagram
  participant UI as 前端
  participant M as TransferManager
  participant C as PeerConnection
  participant S as 对端任务

  UI->>M: pause_transfer(task_id)
  M->>M: status = Paused
  M->>C: send_control(PauseTransfer, task_id)（新流）
  C->>S: PauseTransfer
  S->>S: status = Paused
  Note over M,S: 双方发送/接收循环每 100ms 检查状态<br/>Paused 时原地等待，不再发送新分片
```

### 6.2 取消

```mermaid
sequenceDiagram
  participant UI as 前端
  participant M as TransferManager
  participant S as 对端任务

  UI->>M: cancel_transfer(task_id)
  M->>M: status = Canceled
  M->>S: CancelTransfer
  S->>S: status = Canceled
  Note over M: 发送循环检测到 Canceled → 返回错误<br/>接收循环检测到 Canceled → 返回错误
  Note over M,S: 临时文件与断点保留
```

### 6.3 恢复（发送端）

```mermaid
sequenceDiagram
  participant M as TransferManager
  participant DB as resume_info
  participant S as 对端

  M->>M: status = Transferring
  M->>DB: load_resume(task_id)
  DB-->>M: ResumeInfo（位图）
  M->>S: TransferRequest { resume: true }
  S->>DB: load_resume_info(task_id)
  S-->>M: TransferResponse accepted
  M->>S: DataTransferStart { resume: true }
  S->>S: 按位图计算 missing_chunks
  S-->>M: DataTransferReady { missing_chunks }
  M->>S: 仅发送缺失分片
  M->>S: ResumeTransfer 控制消息（另一条流）
  Note over S: 任务状态再次置 Transferring
```

## 7. 事件时序（前端 ⇄ 后端）

```mermaid
sequenceDiagram
  participant BE as Rust 后端
  participant FE as React

  BE-->>FE: device-online(DeviceInfo)
  BE-->>FE: device-offline(device_id)
  BE-->>FE: transfer-request(TransferRequest)
  FE->>BE: accept_transfer_request / reject_transfer_request
  BE-->>FE: transfer-progress(TransferTaskInfo)
  BE-->>FE: transfer-completed(task_id)
  BE-->>FE: transfer-failed({ task_id, error })
```

## 8. 兼容性契约

| 维度 | 约定 | 当前保障 |
| --- | --- | --- |
| 协议版本 | `1.1.0` | 发现包、`ControlMessage`、`AuthHello` 三处统一校验；数据包无版本字段 |
| 设备 ID | UUID v4，每次启动重新生成 | 无持久化，重启后对端“已知设备”记录会与新 ID 不匹配 |
| 序列化 | bincode 1.3 默认配置 | 无 schema 演进能力，字段顺序/类型变更即不兼容 |
| 端口 | UDP 45678 / QUIC 45679 | 修改需同步 `constants.rs`、Tauri 防火墙说明与文档 |
| 字节序 | 所有长度前缀为大端 `u32` | 两端一致 |
| 最大控制消息 | 10 MiB | 接收端强制校验 |
| 分块上限 | 8 MiB（超大文件） | `optimal_chunk_size` 决定 |
| 证书信任 | 发现包公布 SHA-256 指纹，客户端固定校验 | 旧版本（1.0.0）无法与本版本互通 |

> 1.1.0 是破坏性升级：发现包新增 `cert_fingerprint`、`AuthHello` 新增 `cert_fingerprint`、`ControlMessage` 新增 `protocol_version`，字段顺序变化导致 bincode 不兼容。未来升级需继续同步提升 `PROTOCOL_VERSION`。
