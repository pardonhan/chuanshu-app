# 03 · 核心层与 IPC

## 1. AppState

`src-tauri/src/core/app_state.rs` 定义了全应用共享的运行时状态。

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `devices` | `Arc<DashMap<Uuid, DeviceInfo>>` | 当前在线设备；发现包写入，离线清理移出 |
| `transfer_tasks` | `Arc<DashMap<Uuid, Arc<tokio::sync::Mutex<TransferTask>>>>` | 本次进程创建的所有发送/接收任务，不自动清理 |
| `device_id` | `Uuid` | 本机 ID，首次启动生成并持久化到 `settings.device_id` |
| `device_name` | `Arc<RwLock<String>>` | 可在设置中修改；心跳广播读取最新值 |
| `app_handle` | `Arc<Mutex<Option<tauri::AppHandle>>>` | 后端主动发事件用 |

方法：

| 方法 | 说明 |
| --- | --- |
| `new(device_id, device_name)` | 创建空设备表与任务表 |
| `device_name()` | 读锁失败时回退 `"我的设备"` |
| `set_device_name(name)` | 获取写锁后覆盖 |
| `set_app_handle(handle)` / `get_app_handle()` | 存取 Tauri 句柄 |
| `emit(event, payload)` | 有句柄时 `handle.emit`；无句柄返回 `Ok(())` |
| `emit_async(event, payload)` | `emit` 的 async 别名，供异步任务使用 |

`AppState` 不持有 Tokio Runtime；Runtime 由 `lib.rs` 的 `Arc<Runtime>` 独立持有，仅供关闭窗口时 `block_on` 停止发现服务。

## 2. AppError

`src-tauri/src/core/error.rs` 使用 `thiserror` 定义统一错误，并手写 `Serialize` 输出中文字符串：

| 变体 | 来源 |
| --- | --- |
| `Io(std::io::Error)` | 文件、网络 socket 的同步 I/O |
| `Network(String)` | QUIC 连接、流读写等自定义网络错误 |
| `Serialization(bincode::Error)` | 发现包与传输帧 |
| `JsonSerialization(serde_json::Error)` | 能力列表 JSON |
| `Database(rusqlite::Error)` | SQLite |
| `Uuid(uuid::Error)` | UUID 解析 |
| `Config(String)` | 非法 IP 等配置问题 |
| `DeviceNotFound(String)` | 目标设备不在线 |
| `Permission(String)` | 接收端拒绝传输 |
| `Other(String)` | 其余业务错误 |

`pub type AppResult<T> = Result<T, AppError>`。命令返回 `AppResult<T>`，前端收到的 rejection 就是 `Display` 文本。

## 3. 常量与路径

`src-tauri/src/core/constants.rs`：

| 常量 | 值 |
| --- | --- |
| `DISCOVERY_PORT` | `45678` |
| `DISCOVERY_HEARTBEAT_INTERVAL` | `3`（秒） |
| `DISCOVERY_TIMEOUT` | `30`（秒） |
| `QUIC_IDLE_TIMEOUT` | `60`（秒） |
| `QUIC_KEEP_ALIVE_INTERVAL` | `10`（秒） |
| `QUIC_DEFAULT_PORT` | `45679` |
| `DEFAULT_CHUNK_SIZE` | `4 * 1024 * 1024`（字节） |
| `TRANSFER_PROTOCOL_VERSION` | `2` |
| `PROTOCOL_VERSION` | `"1.0.0"`（仅用于 UDP 发现包） |

`src-tauri/src/core/mod.rs`：

| 函数 | 行为 |
| --- | --- |
| `home_dir()` | 依次读 `USERPROFILE`、`HOME`，都没有则 `"."` |
| `default_download_dir()` | `home_dir()/Downloads/传书` |
| `resolve_download_dir(raw)` | 空串返回默认目录；支持 `~/`、`~\`、单独的 `~`；其余按原样 `PathBuf` |

## 4. 命令清单

`src-tauri/src/lib.rs` 的 `generate_handler!` 恰好注册 15 个命令：

| # | 命令 | 文件 | 签名要点 |
| --- | --- | --- | --- |
| 1 | `get_transfer_tasks` | `ipc/mod.rs` | 返回 `Vec<TransferTaskInfo>`，按 `created_at` 降序 |
| 2 | `send_files` | `ipc/mod.rs` | 入参 `SendFilesRequest`，返回 `Vec<Uuid>` 任务 ID |
| 3 | `cancel_transfer` | `ipc/mod.rs` | `task_id: Uuid` |
| 4 | `pause_transfer` | `ipc/mod.rs` | `task_id: Uuid` |
| 5 | `resume_transfer` | `ipc/mod.rs` | `task_id: Uuid` |
| 6 | `get_settings` | `ipc/mod.rs` | 返回 `Settings`，无记录时给默认值 |
| 7 | `save_settings` | `ipc/mod.rs` | 入参 `Settings`，同时更新内存设备名 |
| 8 | `discover_device_by_ip` | `network/discovery.rs` | `ip: String`，返回 `Option<DeviceInfo>` |
| 9 | `get_transfer_history` | `ipc/mod.rs` | 入参 `TransferHistoryQuery { limit, offset }` |
| 10 | `clear_transfer_history` | `ipc/mod.rs` | 清空表 |
| 11 | `get_known_devices` | `ipc/mod.rs` | 返回 `Vec<KnownDevice>` |
| 12 | `get_system_info` | `ipc/mod.rs` | 返回 `SystemInfo` |
| 13 | `set_auto_launch` | `ipc/mod.rs` | `enabled: bool` |
| 14 | `get_auto_launch` | `ipc/mod.rs` | 返回 `bool` |
| 15 | `get_local_device_info` | `ipc/mod.rs` | 返回 `{ device_id, device_name }` |

### 4.1 传输相关

- `get_transfer_tasks`：遍历 `transfer_tasks`，逐个 `lock().await` 后转成 `TransferTaskInfo`，按 `created_at` 降序返回。任务不会因完成而消失。
- `send_files`：直接调用 `transfer::send_files(request, state)`，内部由 `TransferManager` 收集文件、为每台在线设备创建任务并 `spawn` 发送。
- `cancel_transfer` / `pause_transfer` / `resume_transfer`：调用同名领域函数，只修改任务状态，不做额外 IPC 校验。

### 4.2 设置相关

`Settings` 字段：`device_name`、`download_path`、`auto_accept`、`upload_limit`、`download_limit`、`enable_notification`、`theme`。

- `get_settings`：若 `load_settings()` 返回 `Some` 则原样返回；否则返回默认值——设备名 `"我的设备"`、下载目录 `default_download_dir()`、`auto_accept=true`、限速均为 0、通知开启、主题 `"auto"`。
- `save_settings`：写库后调用 `state.set_device_name(...)`，因此设备名无需重启即可用于后续心跳。

### 4.3 历史与设备

- `get_transfer_history`：需要存储已初始化，否则报 `"Storage not initialized"`；SQL 固定 `ORDER BY created_at DESC`。
- `clear_transfer_history`：`DELETE FROM transfer_history`。
- `get_known_devices`：按 `last_connected DESC, last_seen DESC` 返回，包含离线设备。
- `discover_device_by_ip`：见 [04-network-layer.md](./04-network-layer.md) 的“按 IP 发现”。

### 4.4 系统能力

- `get_system_info`：`os` 取 `std::env::consts::OS`（`windows` / `macos` / `linux`），`arch` 取 `ARCH`；`default_download_path` 为默认下载目录字符串。
  - Windows：通过 PowerShell 查询注册表 `ProductName`，失败返回 `"Unknown"`。
  - macOS：固定返回 `"macOS"`，不读取真实版本。
  - Linux：读 `/etc/os-release` 的 `PRETTY_NAME`，失败返回 `"Linux"`。
- `set_auto_launch`：
  - Windows：写/删 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的 `ChuanshuApp` 值，数据为当前可执行文件路径。
  - macOS：写/删 `~/Library/LaunchAgents/com.chuanshu.app.plist`（`RunAtLoad=true`）。
  - Linux：写/删 `~/.config/autostart/chuanshu.desktop`。
  - 均通过 `std::fs` / `Command` 实现，不检查返回值是否成功（Windows 的 `reg` 输出被忽略）。
- `get_auto_launch`：Windows 执行 `reg query` 并看退出码；macOS/Linux 检查对应文件是否存在。
- `get_local_device_info`：返回内存中的 `device_id` 与当前设备名。

## 5. 事件

后端共发出 5 种事件：

| 事件 | 载荷 | 发射位置 |
| --- | --- | --- |
| `device-online` | `DeviceInfo` | `network/discovery.rs`（新设备、重新上线、IP 发现成功） |
| `device-offline` | `Uuid`（序列化为字符串） | `discovery.rs`（收到 `Goodbye` 或超时清理） |
| `transfer-progress` | `TransferTaskInfo` | `transfer/manager.rs::start_progress_updater` |
| `transfer-completed` | `Uuid`（任务 ID 字符串） | `ipc::emit_transfer_completed` |
| `transfer-failed` | `{ "taskId": Uuid, "error": String }` | `ipc::emit_transfer_failed` |

`ipc/mod.rs` 只把 completed/failed 事件名定义为常量（`TRANSFER_COMPLETED_EVENT`、`TRANSFER_FAILED_EVENT`）；`transfer-progress` 与两个设备事件使用字符串字面量。

`emit_transfer_completed` / `emit_transfer_failed` 在发事件的同时调用 `send_notification`：

- 通知受 `settings.enable_notification` 控制；读不到设置时默认发送。
- 标题分别为“传输完成”“传输失败”，正文包含对端设备名或错误文本。
- 使用 `tauri_plugin_notification` 的 `NotificationExt`。

## 6. 插件与权限

`lib.rs` 注册 4 个插件：`tauri_plugin_fs`、`tauri_plugin_notification`、`tauri_plugin_shell`、`tauri_plugin_dialog`；debug 构建额外注册 `tauri_plugin_log`（级别 `Debug`）。

`src-tauri/capabilities/default.json` 只声明：

```json
["core:default", "dialog:allow-open"]
```

前端因此只能直接使用核心 API 与“选择文件夹”对话框；通知、shell 等能力均由 Rust 侧调用，不经过前端权限。
