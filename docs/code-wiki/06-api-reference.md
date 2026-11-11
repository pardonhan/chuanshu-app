# 06 · API 参考

本文是前后端联调与二次开发时的速查表。所有命令名、参数名、返回类型均取自 `src-tauri/src/ipc/mod.rs` 与 `lib.rs` 的 `generate_handler!`。

> Tauri 2 会把前端传入的 **camelCase** 参数自动映射为 Rust 的 **snake_case** 形参（例如 `taskId` → `task_id`）。

---

## 1. IPC 命令完整列表

共 18 个命令，全部返回 `AppResult<T>`；出错时 Promise reject，错误体为**字符串**。

### 1.1 设备

| # | 命令 | 参数 | 返回 | 说明 |
|---|------|------|------|------|
| 1 | `get_device_list` | — | `DeviceInfo[]` | 仅返回 `is_online()` 为真的运行时设备（**前端未使用**） |
| 2 | `get_known_devices` | — | `KnownDevice[]` | 全部已知设备（含离线），按 `last_connected`/`last_seen` 倒序 |
| 3 | `discover_device_by_ip` | `ip: string` | `DeviceInfo \| null` | 单播发现，3s 超时；成功会写库并 emit `device-online` |
| 4 | `delete_known_device` | `deviceId: string` | `void` | 解析 UUID 后删除（**前端未使用**） |

### 1.2 传输

| # | 命令 | 参数 | 返回 | 说明 |
|---|------|------|------|------|
| 5 | `send_files` | `request: { device_ids: string[]; file_paths: string[] }` | `string[]`（task_id 列表） | 先收集文件元数据，再为每个设备建任务并异步发送 |
| 6 | `get_transfer_tasks` | — | `TransferTaskInfo[]` | 内存任务列表，`created_at` 倒序 |
| 7 | `pause_transfer` | `taskId: string` | `void` | 仅当 `Transferring` 生效 |
| 8 | `resume_transfer` | `taskId: string` | `void` | 仅当 `Paused` 生效，尝试加载续传记录 |
| 9 | `cancel_transfer` | `taskId: string` | `void` | 置 `Canceled`，不通知对端 |
| 10 | `resend_files` | `device_ids: string[]`、`file_paths: string[]` | `string[]` | 从历史重发；**前端封装的参数名不匹配**（见 08） |

### 1.3 历史

| # | 命令 | 参数 | 返回 | 说明 |
|---|------|------|------|------|
| 11 | `get_transfer_history` | `query: { limit: number; offset: number }` | `TransferHistoryEntry[]` | `created_at DESC` |
| 12 | `clear_transfer_history` | — | `void` | 清空全表 |

### 1.4 设置与系统

| # | 命令 | 参数 | 返回 | 说明 |
|---|------|------|------|------|
| 13 | `get_settings` | — | `Settings` | 未保存过时返回默认值（不落库） |
| 14 | `save_settings` | `settings: Settings` | `void` | 写入 7 个 key；**不改运行期设备名** |
| 15 | `get_system_info` | — | `SystemInfo` | OS/版本/架构/默认下载目录 |
| 16 | `set_auto_launch` | `enabled: boolean` | `void` | 写注册表 / LaunchAgent / autostart |
| 17 | `get_auto_launch` | — | `boolean` | 检查对应位置是否存在 |
| 18 | `open_file_location` | `filePath: string` | `void` | 打开**父目录**（不会选中文件） |

### 1.5 注册顺序（`lib.rs`）

```rust
get_device_list, get_transfer_tasks, send_files, cancel_transfer, pause_transfer,
resume_transfer, get_settings, save_settings, discover_device_by_ip,
get_transfer_history, clear_transfer_history, get_known_devices, delete_known_device,
get_system_info, set_auto_launch, get_auto_launch, open_file_location, resend_files
```

---

## 2. 事件（后端 → 前端）

| 事件名 | 载荷结构 | 发射位置 | 前端消费 |
|--------|----------|----------|----------|
| `device-online` | `DeviceInfo` | `discovery.rs`（Announce 新设备、Response 新/曾离线、手动发现） | `useDeviceStore.setDeviceOnline` |
| `device-offline` | `Uuid`（JSON 字符串） | `discovery.rs`（Goodbye、超时、连接关闭回调） | `useDeviceStore.setDeviceOffline` |
| `transfer-progress` | `TransferTaskInfo` | `manager.rs`（100ms）、`quic_server.rs`（100ms）、`start_progress_updater`（500ms） | `useTransferStore.updateTaskProgress` |
| `transfer-completed` | `Uuid`（字符串） | `ipc::emit_transfer_completed` | 重新拉取任务 |
| `transfer-failed` | `{ "task_id": string, "error": string }` | `ipc::emit_transfer_failed` | 期望 `{ taskId, error }`——**字段不匹配** |
| `incoming-transfer` | `{ task_id: string, sender_name: string, file_count: number, total_size: number }` | `ipc::emit_incoming_transfer` | `onIncomingTransfer`，`App.tsx` 弹应用内通知 |

### 系统通知

| 触发 | 标题 | 内容 |
|------|------|------|
| 收到文件 | `收到文件传输` | `来自 {sender_name} 的文件传输` / `来自 {sender_name} 的 N 个文件` |
| 接收完成 | `传输完成` | `文件已从 {device_name} 接收完成` |
| 传输失败 | `传输失败` | `文件传输失败：{error}` |
| 关闭条件 | `settings.enable_notification == false` | 直接返回 |

---

## 3. 数据结构

### 3.1 `DeviceInfo`

```jsonc
{
  "device_id": "uuid-string",
  "device_name": "我的设备",
  "os": "Windows",                 // Windows | MacOS | Linux | Unknown
  "ip_address": "192.168.1.10",
  "quic_port": 45679,
  "protocol_version": "1.0.0",
  "capabilities": ["FolderTransfer", "ResumeTransfer", "MultiDeviceSend"],
  "last_seen": { "secs_since_epoch": 1762300000, "nanos_since_epoch": 0 }
}
```

> `last_seen` 是 `SystemTime` 的 serde 结构体形式，前端 `DeviceInfo.last_seen: number` 与之不符。

### 3.2 `KnownDevice`（SQLite 行）

```jsonc
{
  "device_id": "uuid-string",
  "device_name": "我的设备",
  "os": "Windows",
  "ip_address": "192.168.1.10",
  "quic_port": 45679,
  "protocol_version": "1.0.0",
  "capabilities": "[\"FolderTransfer\",\"ResumeTransfer\"]",  // JSON 字符串
  "last_seen": 1762300000,
  "last_connected": 1762300000,
  "is_online": true,
  "created_at": 1762200000
}
```

### 3.3 `TransferTaskInfo`

```jsonc
{
  "task_id": "uuid",
  "peer_device_name": "同事的 MacBook",
  "transfer_type": "Send",          // Send | Receive
  "status": "Transferring",         // Pending|Transferring|Paused|Completed|Failed|Canceled
  "total_size": 10485760,
  "transferred_size": 5242880,
  "file_count": 3,
  "current_file": "report.pdf",
  "speed": 1258291,                 // bytes/s
  "progress": 50.0,                 // 0–100
  "error_message": null,
  "created_at": 1762300000          // 秒
}
```

### 3.4 `TransferHistoryEntry`

```jsonc
{
  "task_id": "uuid",
  "peer_device_name": "同事的 MacBook",
  "transfer_type": "send",          // 小写
  "status": "completed",            // 小写
  "total_size": 10485760,
  "file_count": 3,
  "file_names": "a.pdf, b.png, c.zip",
  "created_at": 1762300000,
  "completed_at": 1762300060
}
```

> 不包含 `peer_device_id`、`error_message`、`save_path`。

### 3.5 `Settings`

```jsonc
{
  "device_name": "我的设备",
  "download_path": "~/Downloads/传书",
  "auto_accept": false,
  "upload_limit": 0,        // 字节/秒（0=不限）
  "download_limit": 0,      // 字节/秒（0=不限）
  "enable_notification": true,
  "theme": "auto"           // auto | light | dark
}
```

默认值（`get_settings` 无记录时）：

| 字段 | 默认 |
|------|------|
| `device_name` | `"我的设备"` |
| `download_path` | `"~/Downloads/传书"` |
| `auto_accept` | `false` |
| `upload_limit` / `download_limit` | `0` |
| `enable_notification` | `true` |
| `theme` | `"auto"` |

> 存储层实际以**键值对**保存（`settings(key, value)`），`device_name` 不存在时 `load_settings()` 返回 `None`，即使其他 key 存在。

### 3.6 `SystemInfo`

```jsonc
{
  "os": "windows",
  "os_version": "Windows 11 Pro",   // powershell 查注册表；Linux 读 PRETTY_NAME；macOS 固定 "macOS"
  "arch": "x86_64",
  "default_download_path": "C:\\Users\\xxx\\Downloads\\传书"
}
```

### 3.7 协议结构

见 [04 网络层 §2](04-network.md#2-协议-protocolrs)，此处不重复。

---

## 4. 数据库 Schema

文件：`<app_data_dir>/chuanshu.db`（SQLite，WAL 未启用）。

```sql
-- 设置（键值对）
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER DEFAULT (strftime('%s','now'))
);

-- 传输历史（只记录终态）
CREATE TABLE transfer_history (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id TEXT UNIQUE NOT NULL,
  peer_device_id TEXT NOT NULL,
  peer_device_name TEXT NOT NULL,
  transfer_type TEXT NOT NULL,
  status TEXT NOT NULL,
  total_size INTEGER NOT NULL,
  file_count INTEGER NOT NULL,
  file_names TEXT,
  error_message TEXT,
  created_at INTEGER DEFAULT (strftime('%s','now')),
  completed_at INTEGER
);
CREATE INDEX idx_history_status ON transfer_history(status);
CREATE INDEX idx_history_created ON transfer_history(created_at DESC);

-- 续传信息（当前几乎未被写入）
CREATE TABLE resume_info (
  task_id TEXT PRIMARY KEY,
  peer_device_id TEXT NOT NULL,
  peer_device_name TEXT NOT NULL,
  transfer_type TEXT NOT NULL,
  total_size INTEGER NOT NULL,
  total_transferred INTEGER NOT NULL DEFAULT 0,
  files_json TEXT NOT NULL,
  received_chunks_json TEXT NOT NULL,
  save_path TEXT NOT NULL,
  created_at INTEGER DEFAULT (strftime('%s','now')),
  updated_at INTEGER DEFAULT (strftime('%s','now'))
);
CREATE INDEX idx_resume_updated ON resume_info(updated_at);

-- 已知设备
CREATE TABLE known_devices (
  device_id TEXT PRIMARY KEY,
  device_name TEXT NOT NULL,
  os TEXT NOT NULL,
  ip_address TEXT NOT NULL,
  quic_port INTEGER NOT NULL,
  protocol_version TEXT,
  capabilities TEXT,
  last_seen INTEGER DEFAULT (strftime('%s','now')),
  last_connected INTEGER,
  is_online INTEGER DEFAULT 0,
  created_at INTEGER DEFAULT (strftime('%s','now'))
);
CREATE INDEX idx_known_devices_last_connected ON known_devices(last_connected DESC);
```

`settings` 使用的 key：`device_name`、`download_path`、`auto_accept`、`upload_limit`、`download_limit`、`enable_notification`、`theme`、`minimize_to_tray`（布尔以 `"true"/"false"`，数字以十进制字符串存储）。

---

## 5. 事件 / 命令与前端类型对照

| 前端导出 | 后端对应 | 备注 |
|----------|----------|------|
| `DeviceInfo` | `network::device::DeviceInfo` | `last_seen` 类型不一致 |
| `KnownDevice` | `storage::KnownDevice` | `is_online: bool` ↔ SQLite INTEGER |
| `TransferTaskInfo` | `transfer::task::TransferTaskInfo` | `transfer_type` 首字母大写 |
| `TransferHistoryEntry` | `storage::TransferHistoryEntry` | `transfer_type`/`status` 小写 |
| `Settings` | `ipc::Settings` | 一致 |
| `SystemInfo` | `ipc::SystemInfo` | 一致 |
| `SendFilesRequest` | `ipc::SendFilesRequest` | 一致 |
| `TransferHistoryQuery` | `ipc::TransferHistoryQuery` | 一致 |

---

## 6. 错误约定

```rust
type AppResult<T> = Result<T, AppError>;
```

- 前端 `catch (e: any)` 得到的 `e` 是**字符串**（`AppError` 的 `Serialize` 实现为 `to_string()`）。
- 常见错误文案：

| 场景 | 文案示例 |
|------|----------|
| 设置未保存 | `app error: ...`（DB 错误的中文前缀） |
| 存储未初始化 | `Storage not initialized` |
| 设备不存在 | `Device {} not found` |
| 任务不存在 | `Task {} not found`（定义存在但当前未被使用） |
| 网络失败 | `网络错误: ...` / `Connection timeout` |
| 校验失败 | `Chunk {} checksum mismatch: expected {}, got {}` |

> 由于只有字符串，前端无法区分“可重试”和“不可重试”，只能统一提示。

---

## 7. 权限（capabilities）

`src-tauri/capabilities/default.json` 仅包含：

```json
{
  "identifier": "default",
  "windows": ["main"],
  "permissions": ["core:default", "dialog:allow-open", "notification:allow-notify"]
}
```

含义：

- `core:default`：Tauri 核心（窗口、事件、路径等）。
- `dialog:allow-open`：设置页选择下载目录需要。
- **没有** `fs:*`、`shell:*`、`notification:*` 的前端权限——目前前端也确实没有直接调用这些插件（文件读写全部在 Rust 侧完成），所以不报错。若后续前端要用 `plugin-fs` / `plugin-notification`，必须在此追加权限。

---

## 8. 前端调用示例

```ts
// 发送
const taskIds = await invoke<string[]>('send_files', {
  request: { device_ids: [deviceId], file_paths: ['C:\\tmp\\a.pdf'] },
})

// 监听进度
const unlisten = await listen<TransferTaskInfo>('transfer-progress', (e) => {
  console.log(e.payload.task_id, e.payload.progress)
})
// 组件卸载时 unlisten()
```

> `tauriApi.ts` 已封装好上述调用，业务代码优先使用封装而不是直接 `invoke`。
