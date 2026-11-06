# 09 · 参考手册

## 1. 常量与魔法数字

### `core/constants.rs`

| 常量 | 值 | 使用位置 |
| --- | --- | --- |
| `DISCOVERY_PORT` | `45678` | UDP 绑定、广播、定向探测、按 IP 发现 |
| `DISCOVERY_HEARTBEAT_INTERVAL` | `3` s | 心跳广播间隔 |
| `DISCOVERY_TIMEOUT` | `30` s | 离线清理周期与超时阈值 |
| `QUIC_IDLE_TIMEOUT` | `60` s | QUIC `max_idle_timeout` |
| `QUIC_KEEP_ALIVE_INTERVAL` | `10` s | QUIC `keep_alive_interval` |
| `QUIC_DEFAULT_PORT` | `45679` | 服务端监听、发现包中的 `quic_port` |
| `DEFAULT_CHUNK_SIZE` | 4 MB | 发送缓冲区、接收分片上限 |
| `TRANSFER_PROTOCOL_VERSION` | `2` | 传输请求版本校验 |
| `PROTOCOL_VERSION` | `"1.0.0"` | 发现包版本校验 |

### `network/protocol.rs`

| 常量 | 值 | 说明 |
| --- | --- | --- |
| `PROTOCOL_VERSION` | 重导出 `core::constants::PROTOCOL_VERSION` | 避免重复定义 |
| `MAX_CONTROL_FRAME_SIZE` | 8 MB | 请求、响应、文件头的长度上限 |
| `TAG_TRANSFER_END` | `0x00` | 全部文件结束 |
| `TAG_DATA` | `0x01` | 数据帧 |
| `TAG_FILE_END` | `0x02` | 单文件结束 |
| `TAG_FILE_HEADER` | `0x03` | 单文件开始 |

### 其它散落数值

| 数值 | 位置 | 含义 |
| --- | --- | --- |
| 25 s | `discovery.rs::check_device_was_offline` | 判断“曾离线”，用于决定是否重复发上线事件 |
| 30 s | `discovery.rs::KNOWN_DEVICE_PERSIST_INTERVAL` | 已知设备写库节流 |
| 2 s / 10 s | `probe_known_devices` | 启动后延迟 / 探测周期 |
| 50 ms / 500 ms | `probe_known_devices` | 目标间间隔 / 收包轮询超时 |
| 3 s | `discover_device_by_ip` | 等待响应超时 |
| 10 s | `quic_server.rs` | `accept_bi` 超时 |
| 10 s / 15 s / 60 s / 5 s | `quic_client.rs` | 建连 / 等响应 / 等确认 / `wait_idle` |
| 500 ms | `start_progress_updater` | 进度事件周期 |
| 100 ms | `wait_if_paused` | 暂停轮询间隔 |
| 9999 | `unique_path` | 重名文件最大尝试序号 |

## 2. 命令表

`src-tauri/src/lib.rs` 的 `generate_handler!` 共 15 个：

| 命令 | 入参 | 返回 | 所在文件 |
| --- | --- | --- | --- |
| `get_transfer_tasks` | — | `Vec<TransferTaskInfo>` | `ipc/mod.rs` |
| `send_files` | `{ request: { device_ids, file_paths } }` | `Vec<Uuid>` | `ipc/mod.rs` |
| `cancel_transfer` | `{ taskId }` | `()` | `ipc/mod.rs` |
| `pause_transfer` | `{ taskId }` | `()` | `ipc/mod.rs` |
| `resume_transfer` | `{ taskId }` | `()` | `ipc/mod.rs` |
| `get_settings` | — | `Settings` | `ipc/mod.rs` |
| `save_settings` | `{ settings }` | `()` | `ipc/mod.rs` |
| `discover_device_by_ip` | `{ ip }` | `Option<DeviceInfo>` | `network/discovery.rs` |
| `get_transfer_history` | `{ query: { limit, offset } }` | `Vec<TransferHistoryEntry>` | `ipc/mod.rs` |
| `clear_transfer_history` | — | `()` | `ipc/mod.rs` |
| `get_known_devices` | — | `Vec<KnownDevice>` | `ipc/mod.rs` |
| `get_system_info` | — | `SystemInfo` | `ipc/mod.rs` |
| `set_auto_launch` | `{ enabled }` | `()` | `ipc/mod.rs` |
| `get_auto_launch` | — | `bool` | `ipc/mod.rs` |
| `get_local_device_info` | — | `{ device_id, device_name }` | `ipc/mod.rs` |

前端参数名按 Tauri 默认规则使用 camelCase（如 `taskId`）；结构体内部字段保持 Rust 的 snake_case。

## 3. 事件表

| 事件 | 载荷 | 来源 |
| --- | --- | --- |
| `device-online` | `DeviceInfo` | `discovery.rs` |
| `device-offline` | UUID 字符串 | `discovery.rs` |
| `transfer-progress` | `TransferTaskInfo` | `manager.rs` |
| `transfer-completed` | 任务 ID 字符串 | `ipc::emit_transfer_completed` |
| `transfer-failed` | `{ "taskId": 字符串, "error": 字符串 }` | `ipc::emit_transfer_failed` |

不存在“接收请求确认”事件；`auto_accept=false` 时接收端直接拒绝。

## 4. 类型映射

| Rust | 序列化后 | 前端 TS | 备注 |
| --- | --- | --- | --- |
| `Uuid` | 字符串 | `string` | 事件载荷中的 UUID 也是字符串 |
| `OperatingSystem` | `"Windows" / "MacOS" / "Linux" / "Unknown"` | 同名联合 | — |
| `Capability` | `"FolderTransfer" / "MultiDeviceSend" / ...` | `string[]` / `string` | `DeviceInfo.capabilities` 是数组；`KnownDevice.capabilities` 是 JSON 字符串 |
| `TransferStatus` | `"Pending" / "Transferring" / "Paused" / "Completed" / "Failed" / "Canceled"` | 同名联合 | 历史表用另一套小写字符串 |
| `TransferType` | `"Send" / "Receive"` | `'Send' \| 'Receive'` | 历史表用 `"send" / "receive"` |
| `TransferHistoryEntry.created_at` | `i64` Unix 秒 | `number` | 前端乘 1000 |
| `TransferTaskInfo.created_at` | `u64` Unix 秒 | `number` | 由 `SystemTime` 转换 |
| `SystemTime` | 不会直接暴露 | — | 任务只通过 `TransferTaskInfo` 输出 |
| `AppError` | 中文字符串 | `string` / `unknown` | 手写 `Serialize` |
| `Settings.upload_limit` / `download_limit` | 字节/秒 | MB/s | 只在 `tauriApi.ts` 换算 |
| `Option<String>` | `string \| null` | `string \| null` | `reason`、`save_path`、`completed_at` 等 |

## 5. 文件清单

### 前端

| 文件 | 职责 |
| --- | --- |
| `src/main.tsx` | React 挂载 |
| `src/App.tsx` | 主题、布局、页面切换 |
| `src/index.css` | Tailwind 指令与全局样式 |
| `src/services/tauriApi.ts` | 15 个命令封装、5 个事件订阅、类型定义、单位换算 |
| `src/store/useDeviceStore.ts` | 设备列表与在线状态 |
| `src/store/useTransferStore.ts` | 任务列表与进度 |
| `src/pages/HomePage.tsx` | 主页布局与选中设备 |
| `src/pages/HistoryPage.tsx` | 历史表格、清空 |
| `src/pages/SettingsPage.tsx` | 设置表单、主题、自启、系统信息 |
| `src/components/device/DeviceList.tsx` | 已知设备、状态、IP 发现 |
| `src/components/layout/Sidebar.tsx` | 三项导航 |
| `src/components/transfer/FileDropZone.tsx` | 拖拽收集路径、确认发送 |
| `src/components/transfer/TransferList.tsx` | 进度展示与暂停/恢复/取消 |

### 后端

| 文件 | 职责 |
| --- | --- |
| `src-tauri/src/main.rs` | 二进制入口 |
| `src-tauri/src/lib.rs` | Builder、插件、命令注册、启动与退出 |
| `core/app_state.rs` | `AppState`、事件发送 |
| `core/constants.rs` | 端口、超时、协议版本、分片大小 |
| `core/error.rs` | `AppError` / `AppResult` |
| `core/mod.rs` | `home_dir` / `default_download_dir` / `resolve_download_dir` |
| `ipc/mod.rs` | 14 个命令、`Settings`、事件常量、通知、自启、系统信息 |
| `network/discovery.rs` | UDP 发现、离线清理、已知设备探测、按 IP 发现命令 |
| `network/device.rs` | `DeviceInfo` / `KnownDevice` / 枚举 |
| `network/protocol.rs` | 发现包、传输结构、帧读写 |
| `network/tls.rs` | 自签名证书、QUIC 参数、TLS 配置 |
| `network/quic_server.rs` | 监听、接收、校验、落盘、写历史 |
| `network/quic_client.rs` | 建连、发送文件、等待确认 |
| `transfer/manager.rs` | 文件收集、任务创建、暂停/恢复/取消、进度任务 |
| `transfer/task.rs` | `TransferTask` / `TransferTaskInfo` / 状态枚举 |
| `transfer/file_chunk.rs` | `calculate_crc32` |
| `transfer/rate_limiter.rs` | 令牌桶（含 3 个单元测试） |
| `storage/mod.rs` | SQLite 三张表与全部存取方法 |

### 配置

| 文件 | 作用 |
| --- | --- |
| `package.json` | 前端依赖与脚本 |
| `vite.config.ts` | 端口 1420、`dist` 输出、环境变量前缀 |
| `src-tauri/Cargo.toml` | Rust 依赖与最低版本 |
| `src-tauri/tauri.conf.json` | 窗口 1000×700、打包、插件、`devUrl` |
| `src-tauri/capabilities/default.json` | `core:default`、`dialog:allow-open` |
| `entitlements.mac.plist` | macOS 网络与文件权限、隐私说明 |

## 6. 已知限制

| 项 | 说明 |
| --- | --- |
| 无对端身份校验 | 客户端跳过 TLS 证书校验，`sender_id` / `sender_name` 为自报值；能访问端口的主机都可发送文件 |
| 无续传 | 中断后只能重新发送；任务不持久化，重启即丢失 |
| 无接收确认 | 只支持 `auto_accept` 开关；关闭即拒绝，没有逐次确认 UI |
| 暂停为单端行为 | 没有控制帧同步对端；对端只表现为读/写阻塞 |
| 取消不校验状态 | `cancel_transfer` 可把已完成任务改成 `Canceled` |
| 前端取消后短暂显示失败 | 后端取消也发 `transfer-failed`，`TransferList` 收到后先置 `Failed`，下一轮 2 秒轮询才恢复为 `Canceled` |
| 任务不清理 | `transfer_tasks` 随进程增长，`get_transfer_tasks` 返回全部历史任务 |
| 无限并发/排队 | 每次 `send_files` 立即为每台设备建连接并发送 |
| 无自动重试 | 网络抖动直接失败，需用户重新发送 |
| SQLite 同步阻塞 | 在 async 任务中直接执行 SQL，查询大表时占用工作线程 |
| 事件与轮询并存 | 设备 5 s、任务 2 s 轮询与事件双通道，存在冗余 |
| 拖拽监听未注销 | `FileDropZone` 的 effect 未返回 unlisten，切换设备会重复注册 |
| 设备状态两态化 | `scanning` / `connecting` 没有写入路径，界面实际只显示在线/离线 |
| 仅 IPv4 发现 | `discover_device_by_ip`、广播地址枚举都只处理 IPv4 |
| macOS 版本号固定 | `get_system_info` 对 macOS 返回字符串 `"macOS"` |
| Windows 版本依赖 PowerShell | 查询注册表失败时返回 `"Unknown"` |
| 协议不可演进 | bincode + 严格版本比较，字段变更必须同时升级两端 |

## 7. 术语表

| 术语 | 含义 |
| --- | --- |
| **发现（Discovery）** | 通过 UDP 广播与定向包让局域网设备互相可见 |
| **Announce / Response / Goodbye** | 发现协议的三种消息：广播存在、单播回应、退出通知 |
| **KnownDevice** | 持久化在 SQLite 的设备记录，可离线展示并用于跨子网探测 |
| **传输协议 v2** | 当前线格式，一次传输 = 一个 QUIC 连接 + 一条双向流 |
| **帧（Frame）** | 长度前缀 + bincode 结构，或“标签 + 固定字段 + 数据” |
| **标签（Tag）** | `0x00` 全部结束、`0x01` 数据、`0x02` 文件结束、`0x03` 文件头 |
| **分片（Chunk）** | 4 MB 的数据单元，带 CRC32 |
| **任务（Task）** | 一次发送或接收，以 `task_id` 标识，存于内存 |
| **auto_accept** | 接收端是否自动接受传输，默认 `true` |
| **令牌桶** | 限速实现，桶容量与补充速率均为设定字节数/秒，0 表示不限速 |
| **设备四态** | 前端 `scanning / connecting / online / offline` |

## 8. 快速定位

| 想了解 | 直接看 |
| --- | --- |
| 应用如何启动 | `src-tauri/src/lib.rs` 的 `run()` |
| 命令有哪些 | `lib.rs` 的 `generate_handler!` 与 `ipc/mod.rs` |
| 事件从哪里发 | 搜索 `emit(`、`emit_async`、`emit_transfer_completed` |
| 发现如何工作 | `network/discovery.rs` 的 `DiscoveryService::start` |
| 连接如何建立 | `network/quic_client.rs` 的 `send_transfer` |
| 接收如何落盘 | `network/quic_server.rs` 的 `receive_files_inner` |
| 发送如何收集文件 | `transfer/manager.rs` 的 `collect_files` |
| 任务状态在哪里变 | `transfer/task.rs` 与 `transfer/manager.rs` |
| 数据表结构 | `storage/mod.rs` 的 `init_tables` |
| 前后端契约 | `src/services/tauriApi.ts` |
| 界面如何拼装 | `src/App.tsx`、`src/pages/`、`src/components/` |
