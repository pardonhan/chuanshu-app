# 03 · 后端 core 与 IPC

## 1. `core/` 模块

```
core/
├── mod.rs          // 模块导出：error / constants / config / app_state
├── app_state.rs   // 全局共享状态
├── constants.rs   // 端口、超时、块大小等常量
├── error.rs       // AppError / AppResult
└── config.rs      // Settings 与单位转换
```

`core/mod.rs` 只重导出 `error`、`constants`、`config`、`app_state`；配置结构 `Settings` 位于 `config.rs`，由 SQLite 的 `settings` 表持久化。

### 1.1 `AppError`

[error.rs](../../src-tauri/src/core/error.rs) 用 `thiserror` 定义统一错误，并手动实现 `serde::Serialize`，**把错误序列化为中文字符串**，因此前端 `invoke` 失败时拿到的是可读错误文本。

| 变体 | 来源 | 显示文案 |
| --- | --- | --- |
| `Io(std::io::Error)` | `#[from]` | `IO错误: ...` |
| `Network(String)` | 手动构造 | `网络错误: ...` |
| `Serialization(bincode::Error)` | `#[from]` | `序列化错误: ...` |
| `JsonSerialization(serde_json::Error)` | `#[from]` | `JSON序列化错误: ...` |
| `Database(rusqlite::Error)` | `#[from]` | `数据库错误: ...` |
| `Uuid(uuid::Error)` | `#[from]` | `UUID错误: ...` |
| `Config(String)` | 手动 | `配置错误: ...` |
| `DeviceNotFound(String)` | 手动 | `设备未找到: ...` |
| `TaskNotFound` / `Permission` | 已移除（无调用方） | — |
| `Other(String)` | 兜底 | `其他错误: ...` |

`pub type AppResult<T> = Result<T, AppError>`。所有 `#[tauri::command]` 返回 `AppResult<T>`，错误会以字符串 reject 到前端。

### 1.2 `constants.rs`

完整常量表见 [09-reference.md](./09-reference.md#1-常量表)。此处只列影响行为的项：

| 常量 | 值 | 用途 |
| --- | --- | --- |
| `DISCOVERY_PORT` | 45678 | UDP 发现 |
| `DISCOVERY_HEARTBEAT_INTERVAL` | 3 s | 广播间隔 |
| `DISCOVERY_TIMEOUT` | 30 s | 离线判定 |
| `QUIC_DEFAULT_PORT` | 45679 | QUIC 服务端 |
| `QUIC_IDLE_TIMEOUT` | 60 s | 连接空闲超时 |
| `QUIC_KEEP_ALIVE_INTERVAL` | 15 s | 保活 |
| `MAX_CONCURRENT_TRANSFERS` | 5 | 发送队列的信号量上限 |
| `TRANSFER_TASK_RETENTION` | 3600 s | 终态任务在内存中的保留时间 |
| `PROTOCOL_VERSION` | `1.1.0` | 发现包、控制消息、认证统一校验 |

> `SMALL_FILE_THRESHOLD` 仅用于 `FileChunker` 的块大小选择；`MAX_BATCH_*` 与批量传输代码已移除。

### 1.3 `AppState`

[app_state.rs](../../src-tauri/src/core/app_state.rs)

| 方法 | 签名 | 说明 |
| --- | --- | --- |
| `new` | `fn new(device_name: String) -> Self` | 生成随机 `device_id`，初始化所有容器 |
| `set_certificate_fingerprint` | `async fn set_certificate_fingerprint(&self, fingerprint: String)` | 写入本机证书指纹 |
| `certificate_fingerprint` | `async fn certificate_fingerprint(&self) -> Option<String>` | 读取指纹 |
| `set_app_handle` | `fn set_app_handle(&self, handle: AppHandle)` | setup 阶段注入（此时不在异步上下文） |
| `emit_async` | `async fn emit_async<T: Serialize + Clone>(&self, event: &str, payload: T) -> AppResult<()>` | 事件发射 |
| `get_app_handle` | `async fn get_app_handle(&self) -> Option<AppHandle>` | 供传输完成回调使用 |

## 2. `ipc/` 模块

[ipc/mod.rs](../../src-tauri/src/ipc/mod.rs) 是 Tauri 命令与系统能力的集合，提供 **19 个命令**；另有 `discover_device_by_ip` 位于 [network/discovery.rs](../../src-tauri/src/network/discovery.rs)，两者合计 **20 个命令**在 [lib.rs](../../src-tauri/src/lib.rs) 中注册。

### 2.1 数据结构

`Settings` 定义在 [core/config.rs](../../src-tauri/src/core/config.rs)，通过 `crate::core::*` 重导出；`ipc` 与 `storage` 都依赖它，消除了 `storage → ipc` 的反向依赖。

```rust
pub struct SendFilesRequest { pub device_ids: Vec<Uuid>, pub file_paths: Vec<String> }

pub struct Settings {
    pub device_name: String,
    pub download_path: String,
    pub auto_accept: bool,
    pub upload_limit: u32,     // 单位 MB/s（0 = 不限）
    pub download_limit: u32,
    pub enable_notification: bool,
    pub theme: String,         // "auto" | "light" | "dark"
}

pub struct SystemInfo { os, os_version, arch, default_download_path }

pub struct TransferHistoryQuery { pub limit: i64, pub offset: i64 }
```

### 2.2 命令清单

| # | 命令 | 参数 | 返回 | 实现要点 |
| --- | --- | --- | --- | --- |
| 1 | `get_device_list` | — | `Vec<DeviceInfo>` | 只返回 `is_online()`（`last_seen` 在 30s 内）的设备 |
| 2 | `get_transfer_tasks` | — | `Vec<TransferTaskInfo>` | 遍历内存任务表，按 `created_at` 倒序 |
| 3 | `send_files` | `SendFilesRequest` | `Vec<Uuid>` | 委托 `transfer::send_files` |
| 4 | `cancel_transfer` | `task_id` | `()` | 置 `Canceled` 并通知对端 |
| 5 | `pause_transfer` | `task_id` | `()` | 置 `Paused` 并通知对端 |
| 6 | `resume_transfer` | `task_id` | `()` | 置 `Transferring`；发送方会加载断点并重发 |
| 7 | `accept_transfer_request` | `task_id` | `()` | 从 `pending_transfer_requests` 取出 `oneshot::Sender` 并发送 `true` |
| 8 | `reject_transfer_request` | `task_id` | `()` | 同上，发送 `false` |
| 9 | `get_settings` | — | `Settings` | 读 SQLite；无记录时返回默认值 |
| 10 | `save_settings` | `Settings` | `()` | 逐项写入 `settings` 键值表 |
| 11 | `discover_device_by_ip` | `ip: String` | `Option<DeviceInfo>` | 见 [04-network-layer.md](./04-network-layer.md#26-手动-ip-发现)；3 秒超时 |
| 12 | `get_transfer_history` | `TransferHistoryQuery` | `Vec<TransferHistoryEntry>` | `LIMIT/OFFSET` 分页，按创建时间倒序 |
| 13 | `clear_transfer_history` | — | `()` | `DELETE FROM transfer_history` |
| 14 | `get_known_devices` | — | `Vec<KnownDevice>` | 按 `last_connected DESC, last_seen DESC` |
| 15 | `delete_known_device` | `device_id: String` | `()` | 解析 UUID 后删除 |
| 16 | `get_system_info` | — | `SystemInfo` | Windows 查注册表，Linux 读 `/etc/os-release`，macOS 返回 `macOS` |
| 17 | `set_auto_launch` | `enabled: bool` | `()` | Windows 注册表 Run；macOS LaunchAgent；Linux `.desktop` |
| 18 | `get_auto_launch` | — | `bool` | 检查对应自启文件/键是否存在 |
| 19 | `open_file_location` | `file_path: String` | `()` | 取父目录后 `open` / `explorer` / `xdg-open` |
| 20 | `resend_files` | `device_ids: Vec<String>`, `file_paths: Vec<String>` | `Vec<String>` | 解析 ID 后复用 `send_files`；内部构建的 `files` 元数据未被使用 |

### 2.3 事件

事件名常量定义在 `ipc/mod.rs` 末尾，部分事件由 `AppState::emit_async` 直接以字符串发射：

| 事件 | 载荷 | 发射位置 | 前端消费 |
| --- | --- | --- | --- |
| `device-online` | `DeviceInfo` | `discovery.rs`（Announce 新设备；Response 新设备或曾离线；`discover_device_by_ip` 成功） | `DeviceList` |
| `device-offline` | `device_id: Uuid` | `discovery.rs`（Goodbye、30s 超时清理、QUIC 连接关闭回调） | `DeviceList` |
| `transfer-request` | `TransferRequest` | `quic_server.rs` 收到请求且 `auto_accept=false` | `IncomingTransferRequestModal` |
| `transfer-progress` | `TransferTaskInfo` | 发送/接收循环每 ≥100ms | `TransferList` |
| `transfer-completed` | `task_id: Uuid` | 发送成功（`manager.rs`）或接收成功（`quic_server.rs`） | `TransferList` |
| `transfer-failed` | `{ task_id, error }` | 发送失败（`manager.rs`）或接收失败（`quic_server.rs`） | `TransferList` |

### 2.4 系统能力实现

| 能力 | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 开机自启 | `reg add HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v ChuanshuApp` | 写 `~/Library/LaunchAgents/com.chuanshu.app.plist`（`RunAtLoad=true`） | 写 `~/.config/autostart/chuanshu.desktop` |
| 打开目录 | `explorer <dir>` | `open <dir>` | `xdg-open <dir>` |
| 系统版本 | PowerShell 查注册表 `ProductName` | 固定 `macOS` | `/etc/os-release` 的 `PRETTY_NAME` |
| 默认下载路径 | `%USERPROFILE%\Downloads\传书` | `$HOME/Downloads/传书` | `$HOME/Downloads/传书` |

### 2.5 通知

`send_notification` 先读设置里的 `enable_notification`，为 `false` 时直接返回；否则调用 `tauri_plugin_notification` 显示标题与正文。

- `emit_transfer_completed` 会发送「传输完成 / 文件已从 {设备} 接收完成」——该函数实际在**发送方**传输完成后调用，文案与场景不符。
- 接收端完成时只写历史记录，不调用通知函数。

## 3. 命令注册与权限

- 插件在 [lib.rs](../../src-tauri/src/lib.rs#L26-L33) 注册：`tauri_plugin_fs`、`notification`、`shell`、`dialog`，debug 模式额外注册 `tauri_plugin_log`。
- 前端可直接使用的插件能力受 [capabilities/default.json](../../src-tauri/capabilities/default.json) 限制：仅有 `core:default` 与 `dialog:allow-open`。若新增 `shell.open`、通知等前端直调能力，需要同步添加权限。
- 自定义 IPC 命令不受 capabilities 列表约束（它们通过 `invoke_handler` 暴露）。
