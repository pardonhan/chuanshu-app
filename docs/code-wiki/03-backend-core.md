# 03 · 后端核心（入口 / 状态 / IPC / 存储）

本文覆盖 `src-tauri/src` 中除网络与传输算法外的部分：`lib.rs`、`core/*`、`ipc/mod.rs`、`storage/mod.rs`。

---

## 1. 文件与职责

| 文件 | 职责 |
|------|------|
| `main.rs` | 8 行入口，`app_lib::run()`；Windows release 下隐藏控制台（`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`） |
| `lib.rs` | 创建 Tokio Runtime、构造 `AppState`、注册插件/命令、`setup` 中启动三大后台服务、窗口关闭时停机 |
| `core/mod.rs` | 模块聚合与再导出（`error`、`constants`、`app_state`；`config` 被注释） |
| `core/app_state.rs` | 进程级共享状态与事件发射辅助 |
| `core/constants.rs` | 全局常量（端口、超时、块大小、协议版本） |
| `core/error.rs` | `AppError` 枚举 + `AppResult<T>` |
| `core/config.rs` | **空文件**（仅保留 `pub mod config;` 声明） |
| `ipc/mod.rs` | 18 个 `#[tauri::command]`、`Settings`/`SystemInfo`/`SendFilesRequest`/`TransferHistoryQuery` 类型、事件常量与发射函数、系统通知 |
| `storage/mod.rs` | `Storage`（rusqlite 封装）、全局 `STORAGE`、`TransferHistoryEntry`、`KnownDevice` 再导出 |
| `build.rs` | `tauri_build::build()` |

---

## 2. 应用入口 `lib.rs`

```rust
let rt = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));
let app_state = Arc::new(AppState::new("我的设备".to_string()));
```

### 注册内容

| 类别 | 内容 |
|------|------|
| 插件 | `tauri_plugin_fs` / `notification` / `shell` / `dialog`；debug 下额外 `tauri_plugin_log`（`LevelFilter::Debug`） |
| 托管状态 | `Arc<AppState>`、`tokio::runtime::Handle` |
| 命令 | 18 个（见 [06](06-api-reference.md#1-ipc-命令完整列表)） |
| 生命周期 | `setup`、`on_window_event(CloseRequested)` |

### `setup` 顺序

1. debug 下挂载日志插件。
2. `app.path().app_data_dir()` → `storage::init_storage(&dir)`；失败仅 `log::error!`，应用继续运行（后续 `get_storage()` 返回 `None`，多数命令会报 “Storage not initialized” 或返回默认设置）。
3. `storage.load_settings()` 读取 `device_name`，赋给局部变量 **`_device_name`**（下划线开头，未使用）→ **设置中的设备名不会影响运行期 `AppState.device_name`，重启也不生效**（`AppState` 始终是 `"我的设备"`）。
4. `state.set_app_handle(handle)`。
5. `rt_handle.spawn(start_discovery_service(state, handle))`。
6. `rt_handle.spawn(start_quic_server(state))`。
7. `rt_handle.spawn(transfer::start_progress_updater(state))`。

### 关闭流程

```rust
tauri::WindowEvent::CloseRequested { .. } => {
    rt_for_cleanup.block_on(async { network::stop_discovery_service().await.ok(); });
    network::stop_quic_server();
}
```

- 在**主线程**上 `block_on`，会阻塞 UI 直到发现服务停止。
- `stop_quic_server()` 是非 async 函数，只关闭 endpoint。
- 未显式等待/取消 QUIC 服务任务与进度更新任务。

### 一个设计细节

`AppState::new` 每次启动生成新的 `device_id`（`Uuid::new_v4()`），**该 ID 未持久化**。因此同一台机器每次启动在别人眼里都是「新设备」，已知设备表中会不断累积旧记录。见 [08](08-known-issues.md)。

---

## 3. `AppState`

```rust
pub struct AppState {
    pub devices: Arc<DashMap<Uuid, DeviceInfo>>,
    pub transfer_tasks: Arc<DashMap<Uuid, Arc<tokio::sync::Mutex<TransferTask>>>>,
    pub device_id: Uuid,
    pub device_name: String,
    pub app_handle: Arc<tokio::sync::Mutex<Option<tauri::AppHandle>>>,
}
```

| 方法 | 签名 | 说明 |
|------|------|------|
| `new(device_name)` | `-> Self` | 初始化空 DashMap、随机 UUID、`app_handle=None` |
| `set_app_handle` | `(&self, AppHandle)` | `blocking_lock().replace(handle)`；在 `setup` 同步上下文调用，若在 async 上下文调用会 panic |
| `emit_async` | `async fn emit_async<T: Serialize + Clone>(&self, event, payload) -> AppResult<()>` | `lock().await` 后 `handle.emit`；**推荐用法** |
| `emit` | `fn emit<T>(...)` | 使用 `blocking_lock()`，**在 Tokio 工作线程调用会 panic** |
| `get_app_handle` | `-> Option<AppHandle>` | 克隆 handle；同样使用 `blocking_lock` |

### 并发约定

- `devices`：直接 `insert/remove/iter`，无需额外锁。
- `transfer_tasks`：`DashMap` 给的是值级并发，值内部仍需 `tokio::Mutex`；加锁顺序统一为「先取 `Arc`（`get` 克隆），再 `lock().await`」，避免跨 `await` 持有 DashMap 引用（`DashMapRef` 不是 `Send` 友好的跨 await 守卫）。
- `transfer_tasks` 中的任务**不会自动清理**，进程运行期间只增不减（`Completed` 也保留）。

---

## 4. 常量（`core/constants.rs`）

| 常量 | 类型 | 值 | 使用情况 |
|------|------|-----|----------|
| `DISCOVERY_PORT` | `u16` | `45678` | 发现服务 |
| `DISCOVERY_HEARTBEAT_INTERVAL` | `u64` | `3`（秒） | 心跳 + 广播 |
| `DISCOVERY_TIMEOUT` | `u64` | `30`（秒） | 离线判定 |
| `QUIC_IDLE_TIMEOUT` | `u64` | `60`（秒） | QUIC transport |
| `QUIC_KEEP_ALIVE_INTERVAL` | `u64` | `15`（秒） | QUIC transport |
| `QUIC_DEFAULT_PORT` | `u16` | `45679` | 服务端监听、设备公告 |
| `MAX_CONCURRENT_TRANSFERS` | `usize` | `5` | **未使用** |
| `DEFAULT_CHUNK_SIZE` | `u32` | `4 * 1024 * 1024` | 分片 |
| `SMALL_FILE_THRESHOLD` | `u64` | `256 * 1024` | 动态块大小 |
| `MAX_BATCH_FILES` | `usize` | `100` | 批处理（`BatchBuilder`，未接入流程） |
| `MAX_BATCH_SIZE` | `u64` | `64 MB` | 同上 |
| `PROTOCOL_VERSION` | `&str` | `"1.0.0"` | 发现报文与握手 |

---

## 5. 错误类型（`core/error.rs`）

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    Io, Network(String), Serialization(bincode::Error), JsonSerialization(serde_json::Error),
    Database(rusqlite::Error), Uuid(uuid::Error), Config(String), DeviceNotFound(String),
    TaskNotFound(String), Permission(String), Other(String),
}
```

- `#[from]` 自动转换：`std::io::Error`、`bincode::Error`、`serde_json::Error`、`rusqlite::Error`、`uuid::Error`。
- 手写 `impl Serialize`：**序列化为 `self.to_string()` 的纯字符串**，因此前端 `catch (e)` 拿到的是中文字符串（如 `"网络错误: ..."`）。
- Tauri 命令返回 `AppResult<T>`，错误直接以字符串形式 reject。

---

## 6. IPC 层（`ipc/mod.rs`）

### 6.1 类型

```rust
pub struct Settings {
    device_name: String, download_path: String, auto_accept: bool,
    upload_limit: u32, download_limit: u32,
    enable_notification: bool, theme: String,
}
pub struct SystemInfo { os, os_version, arch, default_download_path }
pub struct SendFilesRequest { device_ids: Vec<Uuid>, file_paths: Vec<String> }
pub struct TransferHistoryQuery { limit: i64, offset: i64 }
```

### 6.2 命令实现要点

| 命令 | 实现摘要 |
|------|----------|
| `get_system_info` | `std::env::consts::{OS, ARCH}`；Windows 通过 `powershell` 查注册表 `ProductName`；Linux 读 `/etc/os-release` 的 `PRETTY_NAME`；macOS 固定返回 `"macOS"`；默认路径为 `<home>/Downloads/传书` |
| `set_auto_launch` | Windows：写 `HKCU\...\Run` 的 `ChuanshuApp`；macOS：写 `~/Library/LaunchAgents/com.chuanshu.app.plist`；Linux：写 `~/.config/autostart/chuanshu.desktop` |
| `get_auto_launch` | 对应平台检查注册表项 / plist / desktop 文件是否存在 |
| `get_device_list` | 遍历 `AppState.devices`，仅返回 `is_online()` 为真的设备 |
| `get_transfer_tasks` | 遍历任务表，`TransferTaskInfo::from(&*task)`，按 `created_at` 倒序 |
| `send_files` | 直接转调 `transfer::send_files(...)`，返回 `Vec<Uuid>` |
| `cancel/pause/resume_transfer` | 转调 `transfer::{cancel,pause,resume}_transfer` |
| `get_settings` | 有存储则 `load_settings()`，否则返回默认值（不落库） |
| `save_settings` | `storage.save_settings()`；**不会更新 `AppState.device_name`，也不会重启服务** |
| `get_transfer_history` | `storage.get_transfer_history(limit, offset)` |
| `clear_transfer_history` | `DELETE FROM transfer_history` |
| `get_known_devices` | 返回全部已知设备（含离线） |
| `delete_known_device` | 解析 UUID 后删除 |
| `open_file_location` | 取父目录，Windows `explorer`、macOS `open`、Linux `xdg-open`；**不打开并选中文件** |
| `resend_files` | 构造 `FileMetadata` 后转调 `transfer::send_files`；形参是 `device_ids: Vec<String>` |

### 6.3 事件与通知

```rust
pub const DEVICE_ONLINE_EVENT: &str = "device-online";
pub const DEVICE_OFFLINE_EVENT: &str = "device-offline";
pub const TRANSFER_PROGRESS_EVENT: &str = "transfer-progress";
pub const TRANSFER_COMPLETED_EVENT: &str = "transfer-completed";
pub const TRANSFER_FAILED_EVENT: &str = "transfer-failed";
```

| 函数 | 行为 |
|------|------|
| `emit_device_online(handle, &DeviceInfo)` | emit `device-online` |
| `emit_device_offline(handle, Uuid)` | emit `device-offline`（UUID 序列化为字符串） |
| `emit_transfer_progress(handle, &TransferTaskInfo)` | emit `transfer-progress` |
| `emit_transfer_completed(handle, task_id, device_name)` | emit `transfer-completed` + 通知「文件已从 X 接收完成」 |
| `emit_transfer_failed(handle, task_id, error)` | emit `transfer-failed`，载荷 `{ "task_id": ..., "error": ... }` + 通知 |
| `send_notification(handle, title, body)` | 先读设置 `enable_notification`，关闭则直接返回；否则 `tauri_plugin_notification` |

> 注意：`emit_transfer_failed` 的 JSON key 是 **`task_id`**，而前端 `onTransferFailed` 期望 **`taskId`**，导致前端 `setTaskStatus(undefined, 'Failed')` 无效。见 [08](08-known-issues.md)。

---

## 7. 存储层（`storage/mod.rs`）

### 7.1 初始化与全局实例

```rust
static STORAGE: OnceCell<std::sync::Mutex<Storage>> = OnceCell::new();

pub fn init_storage(app_data_dir: &PathBuf) -> AppResult<()>
pub fn get_storage() -> Option<MutexGuard<'static, Storage>>
```

- 数据库文件：`<app_data_dir>/chuanshu.db`（`app_data_dir` 由 Tauri 提供，Windows 通常为 `%APPDATA%\<identifier>`）。
- `init_storage` 只允许调用一次（`OnceCell::set` 二次返回错误）。
- `get_storage()` 使用 `std::sync::Mutex`，**同步阻塞**；持有 guard 期间不要再次 `get_storage()`，否则自死锁（当前 DAO 没有这种调用）。

### 7.2 表结构

```sql
settings(
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER DEFAULT (strftime('%s','now'))
)

transfer_history(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id TEXT UNIQUE NOT NULL,
  peer_device_id TEXT NOT NULL,
  peer_device_name TEXT NOT NULL,
  transfer_type TEXT NOT NULL,      -- "send" / "receive"
  status TEXT NOT NULL,             -- pending/transferring/paused/completed/failed/canceled
  total_size INTEGER NOT NULL,
  file_count INTEGER NOT NULL,
  file_names TEXT,
  error_message TEXT,
  created_at INTEGER DEFAULT (strftime('%s','now')),
  completed_at INTEGER
)

resume_info(
  task_id TEXT PRIMARY KEY,
  peer_device_id TEXT NOT NULL,
  peer_device_name TEXT NOT NULL,
  transfer_type TEXT NOT NULL,
  total_size INTEGER NOT NULL,
  total_transferred INTEGER NOT NULL DEFAULT 0,
  files_json TEXT NOT NULL,             -- HashMap<u64, FileResumeInfo>
  received_chunks_json TEXT NOT NULL,   -- HashMap<u64, Vec<u8>>
  save_path TEXT NOT NULL,
  created_at INTEGER DEFAULT (strftime('%s','now')),
  updated_at INTEGER DEFAULT (strftime('%s','now'))
)

known_devices(
  device_id TEXT PRIMARY KEY,
  device_name TEXT NOT NULL,
  os TEXT NOT NULL,
  ip_address TEXT NOT NULL,
  quic_port INTEGER NOT NULL,
  protocol_version TEXT,
  capabilities TEXT,                -- JSON array string
  last_seen INTEGER DEFAULT (strftime('%s','now')),
  last_connected INTEGER,
  is_online INTEGER DEFAULT 0,
  created_at INTEGER DEFAULT (strftime('%s','now'))
)
```

索引：`idx_history_status(status)`、`idx_history_created(created_at DESC)`、`idx_resume_updated(updated_at)`、`idx_known_devices_last_connected(last_connected DESC)`。

### 7.3 方法清单

| 分类 | 方法 | 说明 |
|------|------|------|
| 设置 | `save_setting(key, value)` | UPSERT |
| | `get_setting(key)` | `Option<String>` |
| | `save_settings(&Settings)` | 逐项写 7 个 key |
| | `load_settings()` | `device_name` 不存在则返回 `None`（视为未初始化） |
| 历史 | `add_transfer_history(...)` | `ON CONFLICT(task_id) DO UPDATE`，终态时写 `completed_at` |
| | `update_transfer_status(task_id, status, error)` | 终态时写 `completed_at` |
| | `get_transfer_history(limit, offset)` | 按 `created_at DESC` |
| | `clear_transfer_history()` | 全表删除 |
| 续传 | `save_resume_info(&ResumeInfo)` | 序列化 files + chunks；`save_path` 取**第一个文件**的 `temp_path`。仅被 `ResumeManager::save_resume` 调用，而后者无调用方 |
| | `load_resume_info(task_id)` | 反序列化并回填 chunks |
| | `delete_resume_info(task_id)` | — |
| | `list_incomplete_transfers()` | 按 `updated_at DESC` 全量列出（**当前无调用方**） |
| | `cleanup_old_resume_entries(days)` | 按 `updated_at` 清理（**当前无调用方**） |
| 设备 | `save_known_device(&DeviceInfo)` | 把 `Capability` 枚举转字符串数组 JSON；`is_online=1` |
| | `get_known_devices()` | 按 `last_connected DESC, last_seen DESC` |
| | `update_device_online_status(id, bool)` | — |
| | `update_last_connected(id)` | 秒级时间戳 |
| | `mark_device_offline(id)` | `is_online=0` |
| | `mark_all_devices_offline()` | 启动时批量置离线；由 `DiscoveryService::start` 调用 |
| | `delete_known_device(id)` | — |

### 7.4 `TransferHistoryEntry`

```rust
pub struct TransferHistoryEntry {
    pub task_id: String,
    pub peer_device_name: String,
    pub transfer_type: String,   // "send" / "receive"
    pub status: String,
    pub total_size: u64,
    pub file_count: u32,
    pub file_names: Option<String>,
    pub created_at: i64,         // 秒
    pub completed_at: Option<i64>,
}
```

- 查询时**不返回** `peer_device_id`、`error_message`、`save_path`，因此前端无法实现「打开位置」。
- `file_names` 是一个拼接字符串（由调用方决定格式），前端直接作为一列展示。

---

## 8. 后端核心的关键约束（速查）

1. 所有共享状态都在 `AppState`，无其他全局可变状态（除 `OnceCell` 单例）。
2. 命令层很薄：参数校验后转调 `transfer` / `storage`；真正的业务在模块里。
3. 错误统一为字符串，前端无法按错误码分支。
4. 设置只在启动时读一次用于 `device_name`，其余设置每次使用时现读（`auto_accept`、限速、通知开关），因此保存后立即生效（设备名除外）。
5. 存储层是**同步** SQLite，在 async 命令中直接调用会阻塞当前 Tokio worker（数据量小，影响可接受）。
