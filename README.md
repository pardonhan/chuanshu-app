# 传书 (Chuanshu) - 局域网文件传输应用

一款基于 Tauri + React 开发的跨平台局域网文件传输桌面应用。

## 功能特性

### 设备发现与连接

- **自动发现**：基于 UDP 广播的设备自动发现协议，3 秒心跳间隔，30 秒超时检测
- **跨子网发现**：每 10 秒对已知设备做定向探测
- **手动发现**：支持通过输入 IP 地址主动探测指定设备
- **设备列表**：实时显示在线设备，支持刷新和选择
- **跨平台**：支持 Windows、macOS、Linux 设备互相发现和通信

### 文件传输

- **单文件/多文件传输**：支持选择单个或多个文件进行发送
- **文件夹传输**：支持整个文件夹的传输，保持目录结构
- **拖拽发送**：支持拖拽文件到传输区域快速发起传输
- **并发传输**：最大支持 5 个并发传输任务（超出排队）
- **断点续传**：支持传输中断后恢复（元数据不一致时自动全量重传）

### 传输管理

- **进度显示**：实时显示传输进度和速度
- **暂停/恢复**：支持暂停和恢复传输任务
- **取消传输**：支持随时取消正在进行的传输
- **历史记录**：查看已完成/失败的传输历史

### 技术特性

- **QUIC 协议**：基于 QUIC 协议的高效数据传输
- **分块传输**：按文件大小自适应块大小（64KB–8MB）
- **校验和**：小文件（<100MB）计算 SHA-256 整文件校验，分片使用 CRC32
- **证书固定**：自签名证书的 SHA-256 指纹随发现包广播，客户端按指纹校验对端

## 技术架构

### 前端
- **框架**：React 19 + TypeScript
- **UI 组件**：Ant Design 6
- **状态管理**：Zustand
- **样式**：TailwindCSS 4
- **构建工具**：Vite 7

### 后端 (Tauri/Rust)
- **应用框架**：Tauri 2
- **网络协议**：
  - UDP 发现协议（端口 45678）
  - QUIC 传输协议（端口 45679）
- **异步运行时**：Tokio
- **设备 ID**：UUID v4
- **并发数据结构**：DashMap

### 通信协议

#### 设备发现协议
| 字段 | 类型 | 说明 |
|------|------|------|
| message_type | DiscoveryMessageType | Announce/Response/Goodbye |
| protocol_version | String | 协议版本 (1.1.0) |
| device_id | Uuid | 设备唯一标识 |
| device_name | String | 设备显示名称 |
| os | OperatingSystem | 操作系统类型 |
| ip_address | Option<IpAddr> | IP 地址 |
| quic_port | u16 | QUIC 服务端口 |
| capabilities | Vec<Capability> | 设备能力列表 |
| cert_fingerprint | String | QUIC 证书 SHA-256 指纹 |
| timestamp | u64 | 时间戳 |

#### 文件传输协议
| 消息类型 | 说明 |
|----------|------|
| TransferRequest | 文件传输请求 |
| TransferResponse | 传输响应 (接受/拒绝) |
| CancelTransfer | 取消传输 |
| PauseTransfer | 暂停传输 |
| ResumeTransfer | 恢复传输 |
| ProgressUpdate | 进度更新 |

## 项目结构

```
chuanshu-app/
├── src/                          # 前端源码
│   ├── components/
│   │   ├── device/
│   │   │   └── DeviceList.tsx   # 设备列表组件
│   │   ├── transfer/
│   │   │   ├── FileDropZone.tsx # 拖拽区域
│   │   │   └── TransferList.tsx # 传输列表
│   │   └── layout/
│   │       └── Sidebar.tsx      # 侧边栏
│   ├── pages/
│   │   ├── HomePage.tsx         # 主页
│   │   ├── HistoryPage.tsx      # 历史记录页
│   │   └── SettingsPage.tsx     # 设置页
│   ├── services/
│   │   └── tauriApi.ts          # Tauri API 调用
│   ├── store/
│   │   └── useDeviceStore.ts    # 设备状态管理
│   └── App.tsx
├── src-tauri/                    # Tauri/Rust 源码
│   ├── src/
│   │   ├── core/
│   │   │   ├── app_state.rs     # 应用状态
│   │   │   ├── constants.rs     # 常量定义
│   │   │   ├── config.rs        # 配置管理
│   │   │   └── error.rs         # 错误类型
│   │   ├── network/
│   │   │   ├── discovery.rs     # 设备发现服务
│   │   │   ├── device.rs        # 设备信息
│   │   │   ├── protocol.rs      # 协议定义
│   │   │   ├── quic_server.rs   # QUIC 服务端
│   │   │   ├── quic_client.rs   # QUIC 客户端
│   │   │   └── connection.rs    # 连接池管理
│   │   ├── transfer/
│   │   │   ├── manager.rs       # 传输管理器
│   │   │   ├── task.rs          # 传输任务
│   │   │   ├── file_chunk.rs    # 文件分块
│   │   │   └── resume.rs        # 断点续传
│   │   ├── storage/
│   │   │   └── mod.rs           # 本地存储
│   │   ├── ipc/
│   │   │   └── mod.rs           # IPC 命令处理
│   │   ├── lib.rs               # 应用入口
│   │   └── main.rs
│   └── tauri.conf.json          # Tauri 配置
└── package.json
```

## 开发指南

### 环境要求
- Node.js 18+
- pnpm 或 npm
- Rust 1.70+
- Tauri CLI

### 安装依赖
```bash
pnpm install
```

### 启动开发服务器
```bash
# 仅启动前端 Vite 开发服务器
pnpm dev

# 启动 Tauri 桌面应用调试
pnpm tauri dev
```

### 构建发布
```bash
pnpm tauri build
```

## IPC API 命令

| 命令 | 参数 | 返回值 | 说明 |
|------|------|--------|------|
| `get_device_list` | - | `Vec<DeviceInfo>` | 获取在线设备列表 |
| `get_known_devices` | - | `Vec<KnownDevice>` | 获取全部已知设备（含离线） |
| `delete_known_device` | `device_id: String` | `()` | 删除已知设备 |
| `discover_device_by_ip` | `ip: String` | `Option<DeviceInfo>` | 通过 IP 发现设备 |
| `get_transfer_tasks` | - | `Vec<TransferTaskInfo>` | 获取传输任务列表 |
| `send_files` | `SendFilesRequest` | `Vec<Uuid>` | 发送文件 |
| `resend_files` | `device_ids, file_paths` | `Vec<String>` | 从历史记录重新发送 |
| `cancel_transfer` / `pause_transfer` / `resume_transfer` | `task_id: Uuid` | `()` | 取消 / 暂停 / 恢复传输 |
| `accept_transfer_request` / `reject_transfer_request` | `task_id: Uuid` | `()` | 接受 / 拒绝接收请求 |
| `get_settings` / `save_settings` | - / `Settings` | `Settings` / `()` | 读取 / 保存设置 |
| `get_transfer_history` | `TransferHistoryQuery` | `Vec<TransferHistoryEntry>` | 分页查询历史 |
| `clear_transfer_history` | - | `()` | 清空历史 |
| `get_system_info` | - | `SystemInfo` | 系统信息 |
| `set_auto_launch` / `get_auto_launch` | `enabled: bool` / - | `()` / `bool` | 开机自启 |
| `open_file_location` | `file_path: String` | `()` | 打开文件所在目录 |

## 配置说明

### 默认设置
```json
{
  "device_name": "我的设备",
  "download_path": "~/Downloads/传书",
  "auto_accept": false,
  "upload_limit": 0,
  "download_limit": 0,
  "enable_notification": true,
  "theme": "auto"
}
```

### 网络端口
- **DISCOVERY_PORT**: 45678 (UDP 设备发现)
- **QUIC_DEFAULT_PORT**: 45679 (QUIC 传输)

### 超时配置
- **DISCOVERY_HEARTBEAT_INTERVAL**: 3 秒
- **DISCOVERY_TIMEOUT**: 30 秒
- **DISCOVERY_REJOIN_TIMEOUT**: 25 秒
- **QUIC_IDLE_TIMEOUT**: 60 秒
- **QUIC_KEEP_ALIVE_INTERVAL**: 15 秒

### 并发与协议
- **MAX_CONCURRENT_TRANSFERS**: 5（超出排队）
- **PROTOCOL_VERSION**: 1.1.0（与 1.0.0 不兼容）

## 设备能力

| 能力 | 说明 |
|------|------|
| `FolderTransfer` | 支持文件夹传输 |
| `ResumeTransfer` | 支持断点续传 |
| `MultiDeviceSend` | 支持多设备同时发送 |

## 许可证

MIT
