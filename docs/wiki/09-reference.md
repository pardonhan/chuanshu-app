# 09 · 参考手册

## 1. 常量表

### `core/constants.rs`

| 常量 | 值 | 使用情况 |
| --- | --- | --- |
| `DISCOVERY_PORT` | `45678` | UDP 发现端口 |
| `DISCOVERY_HEARTBEAT_INTERVAL` | `3` 秒 | 广播间隔 |
| `DISCOVERY_TIMEOUT` | `30` 秒 | 离线判定与清理周期 |
| `DISCOVERY_REJOIN_TIMEOUT` | `25` 秒 | 判定“曾离线/需要重连” |
| `QUIC_IDLE_TIMEOUT` | `60` 秒 | 连接空闲超时 |
| `QUIC_KEEP_ALIVE_INTERVAL` | `15` 秒 | 保活间隔 |
| `QUIC_DEFAULT_PORT` | `45679` | QUIC 服务端口与发现包中声明的端口 |
| `MAX_CONCURRENT_TRANSFERS` | `5` | 发送任务信号量上限 |
| `TRANSFER_TASK_RETENTION` | `3600` 秒 | 终态任务保留时间 |
| `SMALL_FILE_THRESHOLD` | `256 KiB` | `FileChunker` 选择块大小 |
| `PROTOCOL_VERSION` | `"1.1.0"` | 发现/控制/认证统一校验 |

### `network/protocol.rs`

| 符号 | 值 | 说明 |
| --- | --- | --- |
| `PROTOCOL_VERSION` | 从 `core::constants` 重导出 | 不再重复定义 |
| `cert_fingerprint` 字段 | `DiscoveryPacket` / `AuthHello` | 证书指纹用于固定校验 |

## 2. Rust ↔ TypeScript 类型映射

| Rust | 序列化后的 JSON | TS 声明 | 备注 |
| --- | --- | --- | --- |
| `Uuid` | 字符串 | `string` | — |
| `OperatingSystem` | `"Windows" / "MacOS" / "Linux" / "Unknown"` | 同名联合 | 匹配 |
| `Capability` | `"FolderTransfer" / ...` | `string[]`（设备）/ `string` JSON（已知设备） | `KnownDevice.capabilities` 是二次编码的 JSON 字符串 |
| `TransferStatus` | `"Pending" / "Transferring" / ...` | 同名联合 | 匹配 |
| `TransferType` | `"Send" / "Receive"` | `'Send' \| 'Receive'` | 匹配 |
| `SystemTime` | `{ secs_since_epoch, nanos_since_epoch }` | `number` | **不匹配**：`DeviceInfo.last_seen` 实际是对象；`KnownDevice.last_seen` 才是数字 |
| `PathBuf` | 字符串 | `string \| null` | — |
| `AppError` | 中文字符串 | `unknown`/`string` | 手动实现 `Serialize` |
| `Settings.upload_limit` | `number` | `number` | 单位 MB/s；后端换算为 B/s |
| `TransferHistoryEntry.created_at` | `number`（Unix 秒） | `number` | — |

## 3. 源码文件清单

### 前端（约 2,000 行）

| 文件 | 行数 | 职责 |
| --- | --- | --- |
| `src/main.tsx` | 10 | React 挂载 |
| `src/App.tsx` | 115 | 根组件、主题、页面切换 |
| `src/index.css` | — | 全局样式 |
| `src/services/tauriApi.ts` | 244 | IPC/事件/类型 |
| `src/store/useDeviceStore.ts` | 160 | 设备状态 |
| `src/store/useTransferStore.ts` | 72 | 任务状态 |
| `src/pages/HomePage.tsx` | 36 | 传输主页 |
| `src/pages/HistoryPage.tsx` | 239 | 历史表格 |
| `src/pages/SettingsPage.tsx` | 329 | 设置表单 |
| `src/components/device/DeviceList.tsx` | 219 | 设备列表 |
| `src/components/layout/Sidebar.tsx` | 53 | 侧边栏 |
| `src/components/transfer/FileDropZone.tsx` | 236 | 拖拽发送 |
| `src/components/transfer/TransferList.tsx` | 218 | 任务列表 |
| `src/components/transfer/IncomingTransferRequestModal.tsx` | 77 | 接收确认弹窗 |

### 后端（约 6,000 行）

| 文件 | 行数 | 职责 |
| --- | --- | --- |
| `src-tauri/src/main.rs` | 6 | 二进制入口 |
| `src-tauri/src/lib.rs` | 119 | Builder/插件/命令/启动 |
| `core/app_state.rs` | 80 | 全局状态 |
| `core/constants.rs` | 22 | 常量 |
| `core/error.rs` | 42 | 错误类型 |
| `core/config.rs` | 45 | `Settings` 与单位转换 |
| `ipc/mod.rs` | 511 | 19 个命令（另 1 个在网络层）+ 系统能力 |
| `network/discovery.rs` | 880 | UDP 发现/探测/事件 |
| `network/device.rs` | 60 | 设备模型 |
| `network/protocol.rs` | 367 | 协议结构 |
| `network/auth.rs` | 269 | 认证握手 |
| `network/connection.rs` | 397 | 连接池/证书固定/TLS |
| `network/quic_server.rs` | 967 | 服务端与落盘 |
| `network/quic_client.rs` | 166 | 共享客户端 |
| `transfer/manager.rs` | 904 | 发送管线 |
| `transfer/task.rs` | 153 | 任务模型 |
| `transfer/file_chunk.rs` | 53 | 块大小/校验 |
| `transfer/resume.rs` | 180 | 断点位图 |
| `transfer/rate_limiter.rs` | 148 | 令牌桶（含 3 个测试） |
| `storage/mod.rs` | 612 | SQLite |

## 4. 问题修复记录与剩余限制

### 4.1 已修复（本次变更）

| # | 原问题 | 修复方式 |
| --- | --- | --- |
| 1 | 限速单位 MB/s ↔ B/s | `Settings` 增加 `upload_bytes_per_sec` / `download_bytes_per_sec`，发送/接收统一换算 |
| 2 | 自签证书被 rustls 拒绝 | 新增 `PinnedCertVerifier`，按发现包公布的 SHA-256 指纹校验对端证书 |
| 3 | `blocking_lock` 在 async 中调用 | `AppState` 的证书/句柄访问改为 `async` + `tokio::sync::Mutex` |
| 4 | 设备名设置不生效 | `AppState` 在 `setup` 中读取 SQLite 设置后创建 |
| 5 | `transfer-failed` 字段不匹配 | 前端统一解构 `task_id` |
| 6 | 接收完成不发事件 | 接收成功/失败均 emit `transfer-completed` / `transfer-failed` 并弹通知 |
| 7 | 续传无元数据校验 | `ResumeInfo::reconcile_files` 比较文件名/大小/修改时间/相对路径/校验和 |
| 8 | 无证书固定 | 见 #2；身份签名仍是 nonce 摘要，但已与固定证书绑定 |
| 9 | `receive_window` 仅 16 字节 | 调整为 16 MiB |
| 10 | 每次请求新建 `Endpoint` | 全局共享 `Arc<QuicClient>`，退出时统一关闭 |
| 11 | `device_name` 不可变 | 启动时从设置创建，重启生效 |
| 12 | 任务只增不减 | `start_task_maintenance` 每 60 秒清理超过 1 小时的终态任务 |
| 13 | 无并发上限 | `AppState.transfer_semaphore` 限制 5 个并发发送 |
| 14 | 双进度通道 | 删除 500ms 进度更新器，仅保留传输循环内 100ms 事件 |
| 15 | 事件 + 轮询双通道 | 移除前端 2s/5s 轮询，改用事件驱动 |
| 16 | 连接池 `pending` 占位 | 改为按设备连接锁 `connect_locks` |
| 17 | 两个 `FileHeader` 定义 | 统一到 `network/protocol.rs::FileHeader` |
| 18 | 协议无版本字段 | `ControlMessage` 与 `AuthHello` 增加版本校验，统一升至 1.1.0 |
| 19 | `storage` ↔ `ipc` 循环 | `Settings` 迁到 `core/config.rs` |
| 20 | 同步 SQLite 阻塞 async | 仍为同步调用（见 4.2），但已移除长持有锁的热点 |
| 21 | 临时文件命名冲突 | 改为 `.{task_id}-{file_id}.tmp` |
| 22 | 取消/失败不清理 | 取消时删除临时文件与断点；失败保留以便续传 |
| 23–36 | 空 `config.rs`、死代码、未使用协议结构、UI 细节、文档漂移 | 删除 `FileAssembler` / `BatchBuilder` / `batch_files` / `StreamHandle` / `ProgressUpdate` / `MAX_UDP_PACKET_SIZE` 等；实现打开位置、连接中状态、拖拽清理；修正 `CLAUDE.md` |

### 4.2 仍然存在的限制

| 项 | 说明 |
| --- | --- |
| 同步 SQLite 访问 | `Storage` 仍通过全局 `std::sync::Mutex` 同步执行 SQL；大历史查询会短暂阻塞 Tokio 工作线程 |
| 身份签名强度 | `SHA256(nonce ‖ device_id)` 不是数字签名；安全性依赖 TLS 证书固定与局域网信任模型 |
| 首次发现 TOFU | UDP 发现本身未认证，首次学到的指纹可能被中间人投毒 |
| 无客户端证书校验 | 服务端未启用双向 TLS，`AuthHello` 中的客户端指纹为自报值 |
| 无断点任务列表 | 重启后没有 UI/命令列出未完成任务，只能重新发送触发续传 |
| 无退避/优先级 | 断线重连无指数退避；传输无优先级队列界面 |
| 大 chunk 依赖 | 分块索引依赖两端 `optimal_chunk_size` 一致；协议版本已强制匹配 |

## 5. 与仓库既有文档的差异

| 既有文档 | 描述 | 实际实现 | 结论 |
| --- | --- | --- | --- |
| `README.md` | 设备发现超时 9 秒 | 已更新为 30 秒 | ✅ 已修正 |
| `README.md` | 最大 5 个并发传输 | 已通过 `transfer_semaphore` 实现 | 已一致 |
| `README.md` | 自签名证书保证传输安全 | 已实现证书指纹固定；首次发现仍为 TOFU | 部分夸大 |
| `README.md` | IPC 命令表 | 已补齐 20 个命令 | ✅ 已修正 |
| `README.md` | 小文件 <100MB 计算 SHA-256 | 与实现一致 | ✅ |
| `FEATURES.md` | 设置持久化为 JSON 文件 | 已更新为 SQLite `settings` 键值表 | ✅ 已修正 |
| `FEATURES.md` | 断点续传客户端部分完成 | 已支持元数据校验与发送端重发；仍无任务列表 UI | 基本一致 |
| `DESIGN.md` | 深色模式「未实现」 | 已更新为 antd `darkAlgorithm` 已实现 | ✅ 已修正 |
| `docs/superpowers/.../transfer-reliability-design.md` | 续传需校验任务/文件/大小/时间/校验和 | 已通过 `reconcile_files` 实现 | ✅ 基本完成 |
| `docs/superpowers/.../transfer-reliability-design.md` | 认证不得使用 mock 指纹 | 客户端已拒绝 `mock-` 前缀；服务端使用真实证书指纹 | ✅ 基本完成 |
| `CLAUDE.md` | `pnpm clean` 可清理依赖 | 已改为手动删除说明 | ✅ 已修正 |
| `TECH-STACK-WINDOWS.md` | 依赖含 `xxhash-rust`、`confy`、`anyhow`、`tracing` | 已从 `Cargo.toml` 移除 | ✅ 已清理 |

## 6. 术语表

| 术语 | 含义 |
| --- | --- |
| **发现（Discovery）** | 通过 UDP 广播/定向包让同一局域网设备互相可见 |
| **Announce / Response / Goodbye** | 发现协议的三种消息：广播存在、单播回应、退出告知 |
| **认证（Auth）** | QUIC 连接第一流上的 Hello/Response/Ack 三步握手 |
| **连接池** | `device_id → 已认证 QUIC 连接` 的全局缓存 |
| **控制消息** | 长度前缀 + bincode 的消息封装，承载请求/响应/暂停/取消等 |
| **数据流** | 认证连接上一条专用于文件内容传输的双向流 |
| **分片（Chunk）** | 文件按 64KB–8MB 切分的最小传输单元，带 CRC32 |
| **位图（Bitmap）** | 每分片 1 bit 的已接收记录，用于断点续传 |
| **任务（Task）** | 一次发送或接收，`TransferTask`，以 `task_id` 标识 |
| **已知设备（KnownDevice）** | 持久化在 SQLite 的历史设备记录，可离线展示与跨子网探测 |
| **四态** | 前端设备状态 `scanning / connecting / online / offline` |

## 7. 快速定位源码

| 想了解 | 直接看 |
| --- | --- |
| 应用怎么启动 | [`lib.rs`](../../src-tauri/src/lib.rs) `run()` |
| 命令有哪些 | [`ipc/mod.rs`](../../src-tauri/src/ipc/mod.rs) + `generate_handler!` |
| 事件从哪发 | 搜索 `emit_async` / `emit(` |
| 发现怎么工作 | [`discovery.rs`](../../src-tauri/src/network/discovery.rs) `DiscoveryService::start` |
| 怎么连上对端 | [`quic_client.rs`](../../src-tauri/src/network/quic_client.rs) `send_transfer_request` |
| 握手细节 | [`auth.rs`](../../src-tauri/src/network/auth.rs) |
| 接收怎么落盘 | [`quic_server.rs`](../../src-tauri/src/network/quic_server.rs) `receive_files` |
| 发送怎么分块 | [`manager.rs`](../../src-tauri/src/transfer/manager.rs) `send_files_data` |
| 断点怎么存 | [`storage/mod.rs`](../../src-tauri/src/storage/mod.rs) `save_resume_info` / [`resume.rs`](../../src-tauri/src/transfer/resume.rs) |
| 界面怎么拼 | [`App.tsx`](../../src/App.tsx) + `pages/` + `components/` |
| 前后端契约 | [`tauriApi.ts`](../../src/services/tauriApi.ts) |
