# 06 · 存储

## 1. 数据库位置

`src-tauri/src/storage/mod.rs` 用 `rusqlite`（`bundled` 特性）访问 SQLite。

- 初始化：`lib.rs` 的 setup 中调用 `storage::init_storage(&app_data_dir)`。
- 数据目录：优先读环境变量 `CHUANSHU_DATA_DIR`，否则用 Tauri 的 `app.path().app_data_dir()`。
  - Windows 通常为 `%APPDATA%\<identifier>`；
  - macOS 通常为 `~/Library/Application Support/<identifier>`；
  - Linux 通常为 `~/.local/share/<identifier>`；
  - 其中 `identifier = com.chuanshu.app`（见 `tauri.conf.json`）。
- 数据库文件：`<app_data_dir>/chuanshu.db`。
- 初始化时先 `create_dir_all`，再 `Connection::open`，随后 `init_tables()`。
- 全局句柄：`static STORAGE: OnceCell<Mutex<Storage>>`；`init_storage` 重复调用返回“Storage already initialized”，`get_storage()` 返回 `Option<MutexGuard<'static, Storage>>`。

`CHUANSHU_DATA_DIR` 的主要用途是同机双开测试：给第二个实例指定独立目录，避免设备 ID、设置与数据库互相覆盖。

## 2. 表结构

### settings

```sql
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER DEFAULT (strftime('%s','now'))
);
```

所有值都以字符串保存，读取时逐项 `parse`。常用键：

| key | 值类型 | 说明 |
| --- | --- | --- |
| `device_id` | UUID 字符串 | 首次启动生成，之后保持不变 |
| `device_name` | 字符串 | 设备显示名 |
| `download_path` | 字符串 | 可为空、`~/...` 或绝对路径 |
| `auto_accept` | `true` / `false` | 默认 `true` |
| `upload_limit` | 整数字符串 | 字节/秒 |
| `download_limit` | 整数字符串 | 字节/秒 |
| `enable_notification` | `true` / `false` | 默认 `true` |
| `theme` | `auto` / `light` / `dark` | 默认 `auto` |

`save_settings()` 只写后 7 个键，不改 `device_id`。

### transfer_history

```sql
CREATE TABLE IF NOT EXISTS transfer_history (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id TEXT UNIQUE NOT NULL,
  peer_device_id TEXT NOT NULL,
  peer_device_name TEXT NOT NULL,
  transfer_type TEXT NOT NULL,       -- send / receive
  status TEXT NOT NULL,              -- completed / failed / canceled
  total_size INTEGER NOT NULL,
  file_count INTEGER NOT NULL,
  file_names TEXT,
  error_message TEXT,
  created_at INTEGER DEFAULT (strftime('%s','now')),
  completed_at INTEGER
);
```

索引：`idx_history_status(status)`、`idx_history_created(created_at DESC)`。

### known_devices

```sql
CREATE TABLE IF NOT EXISTS known_devices (
  device_id TEXT PRIMARY KEY,
  device_name TEXT NOT NULL,
  os TEXT NOT NULL,
  ip_address TEXT NOT NULL,
  quic_port INTEGER NOT NULL,
  protocol_version TEXT,
  capabilities TEXT,                 -- JSON 数组字符串
  last_seen INTEGER DEFAULT (strftime('%s','now')),
  last_connected INTEGER,
  is_online INTEGER DEFAULT 0,
  created_at INTEGER DEFAULT (strftime('%s','now'))
);
```

索引：`idx_known_devices_last_connected(last_connected DESC)`。

`init_tables()` 还会执行一条 `DROP TABLE IF EXISTS`，清理旧版本遗留的传输进度表，保证升级后数据库只保留上述三张表。

## 3. 设置 API

| 方法 | 行为 |
| --- | --- |
| `save_setting(key, value)` | `INSERT ... ON CONFLICT(key) DO UPDATE`，同时刷新 `updated_at` |
| `get_setting(key)` | 单值查询，不存在返回 `None` |
| `save_settings(&Settings)` | 依次写 7 个键，无事务包裹 |
| `load_settings()` | 如果 `device_name` 不存在直接返回 `None`；否则组装 `Settings` |
| `get_or_create_device_id()` | 读 `device_id` 并解析为 `Uuid`；缺失或非法则生成新 UUID 并写库 |

`load_settings()` 的缺省规则：

- `download_path` → `"~/Downloads/传书"`
- `auto_accept` → `true`
- `enable_notification` → `true`
- `theme` → `"auto"`
- 两个限速 → `0`（不限速）
- 布尔值解析失败按默认值；整数解析失败按 `0`

命令 `get_settings` 在 `load_settings()` 返回 `None` 时再给一套内存默认值（设备名“我的设备”、下载目录取 `default_download_dir()`），但不会自动写库；只有用户保存设置后才落盘。

## 4. 历史 API

| 方法 | 行为 |
| --- | --- |
| `add_transfer_history(...)` | 以 `task_id` 为唯一键 upsert；状态进入 `completed/failed/canceled` 时把 `completed_at` 设为当前秒，否则置 `NULL` |
| `update_transfer_status(task_id, status, error)` | 更新状态与错误信息；终态时补 `completed_at` |
| `get_transfer_history(limit, offset)` | 按 `created_at DESC` 分页，返回 `TransferHistoryEntry` |
| `clear_transfer_history()` | `DELETE FROM transfer_history` |

`TransferHistoryEntry` 字段：`task_id`、`peer_device_name`、`transfer_type`、`status`、`total_size`、`file_count`、`file_names`、`created_at`、`completed_at`。数据库里的 `peer_device_id` 不返回给前端。

发送任务由 `transfer::manager::record_history()` 写入，接收任务由 `quic_server::save_history()` 写入；两者使用相同的状态字符串。

## 5. 已知设备 API

| 方法 | 行为 |
| --- | --- |
| `save_known_device(&DeviceInfo)` | upsert，写入名称、OS、IP、端口、协议版本、能力 JSON、`last_seen=now`，并把 `is_online` 置 1 |
| `get_known_devices()` | 返回全部记录，排序 `last_connected DESC, last_seen DESC` |
| `update_last_connected(device_id)` | 手动按 IP 发现成功时刷新 |
| `mark_device_offline(device_id)` | `is_online = 0` |
| `mark_all_devices_offline()` | 启动时把所有在线标记清零 |

`Capability` 枚举在写库时转成字符串数组再 `serde_json::to_string`；`OperatingSystem` 转成 `"Windows"` / `"MacOS"` / `"Linux"` / `"Unknown"`。

## 6. 没有续传

本项目**不支持续传**：

- 没有续传相关的模块、命令、数据表或持久化进度；启动时还会主动删除旧版本遗留的进度表。
- 传输任务只存在于 `AppState.transfer_tasks` 内存中，应用重启后全部丢失，`get_transfer_tasks` 只会返回本次进程产生的任务。
- 网络中断、对端退出、应用崩溃或用户取消后，接收端会删除当前未完成的 `.part` 临时文件；已经完成的文件保留。重新传输会从头开始。
- `pause_transfer` / `resume_transfer` 只是同一进程内暂停读写循环，不涉及跨进程或跨连接的进度恢复。

## 7. 并发与限制

- `Storage` 内部是同步 `rusqlite::Connection`，外层用 `std::sync::Mutex`。所有数据库调用都在 async 任务中直接执行，属于阻塞调用；单次查询通常很快，但大历史表或磁盘慢时会短暂占用 Tokio 工作线程。
- 写设置没有事务，逐键 upsert；中途失败可能留下部分新值。
- 表结构通过 `CREATE TABLE IF NOT EXISTS` 建立，没有版本号与迁移框架；修改列定义不会自动作用于已有数据库。
- `get_known_devices()` 不做分页，设备很多时会把全部记录读入内存。
