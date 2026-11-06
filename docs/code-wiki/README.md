# 传书 (Chuanshu) Code Wiki

> 局域网文件传输桌面应用 · Tauri 2 + React 19 + Rust
> 文档基于仓库当前代码（v0.1.0）实际实现整理，非设计意图的推测。

---

## 1. 项目是什么

传书是一款跨平台（Windows / macOS / Linux）局域网文件传输桌面应用：

- **设备发现**：UDP 广播（端口 `45678`），3 秒心跳、30 秒超时，另有「已知设备探测」用于跨子网发现。
- **文件传输**：基于 QUIC（quinn，端口 `45679`），4MB 分片，分片级 CRC32 校验，小文件整体 SHA-256 校验。
- **桌面壳**：Tauri 2；Web 前端通过 `invoke` 调用 Rust 命令，通过事件（`device-online` 等）接收后端推送。
- **本地持久化**：SQLite（`chuanshu.db`），保存设置、传输历史、断点续传信息、已知设备。

项目当前处于 **v0.1.0 原型可用但不完整** 的阶段，详见 [已知问题与实现状态](08-known-issues.md)。

---

## 2. 文档导航

| 文档 | 内容 | 适合谁 |
|------|------|--------|
| [01 整体架构](01-architecture.md) | 分层、运行时模型、进程/线程、端口拓扑、主链路数据流、模块依赖关系 | 所有人（先读这篇） |
| [02 前端模块](02-frontend.md) | React 组件树、页面、Zustand Store、Tauri API 封装、主题机制 | 前端 / 全栈 |
| [03 后端核心](03-backend-core.md) | 入口 `lib.rs`、`AppState`、常量、错误类型、IPC 命令层、SQLite 存储层 | Rust / 后端 |
| [04 网络层](04-network.md) | 协议结构、设备发现、QUIC 服务端/客户端、连接池、认证握手 | 网络 / 后端 |
| [05 传输层](05-transfer.md) | 传输管理器、任务状态机、分片与组装、断点续传、限速器、线上帧格式 | 协议 / 后端 |
| [06 API 参考](06-api-reference.md) | IPC 命令表、事件表、数据结构字段表、数据库 Schema、默认配置 | 前后端联调 |
| [07 构建与运行](07-build-and-run.md) | 环境要求、常用命令、配置文件、CI 工作流、端口与防火墙、排障 | 新人上手 / DevOps |
| [08 已知问题](08-known-issues.md) | 功能实现状态矩阵、缺陷与风险、技术债、后续建议 | 维护者 |
| [09 MVP 改造方案](09-mvp-refactor-plan.md) | 面向"局域网直传"的减负方案：删除认证/连接池/续传，单连接单流协议 v2，跳过证书校验 | 维护者 / 后续开发者 |

推荐阅读顺序：`README → 01 → 06 → 02/03 → 04 → 05 → 08 → 09`。

---

## 3. 技术栈

| 层 | 技术 | 版本（package.json / Cargo.toml） |
|----|------|-----------------------------------|
| 桌面框架 | Tauri | `2.x`（CLI `2.10.1`） |
| 前端框架 | React + TypeScript | `19.2.x` + `5.9.x` |
| UI 组件 | Ant Design / @ant-design/icons | `6.3.3` / `6.1.0` |
| 样式 | Tailwind CSS + PostCSS + autoprefixer | `4.2.2` / `8.5.8` |
| 状态管理 | Zustand | `5.0.12` |
| 构建 | Vite | `7.3.1` |
| 后端语言 | Rust | edition 2021，MSRV `1.77.2` |
| 异步运行时 | Tokio | `1.35`（features = full） |
| QUIC | quinn | `0.10` |
| TLS / 证书 | rustls `0.21` + rcgen `0.12` | — |
| 数据库 | rusqlite | `0.29`（bundled） |
| 并发容器 | dashmap | `5.5` |
| 序列化 | serde / serde_json / bincode | `1.x` / `1.0` / `1.3` |
| 校验 | sha2 / crc32fast / xxhash-rust | `0.10` / `1.3` / `0.8` |
| 文件遍历 | walkdir | `2.4` |

---

## 4. 仓库结构

```text
chuanshu-app/
├── index.html                    # Vite 入口 HTML
├── package.json                  # 前端依赖与脚本
├── vite.config.ts                # dev server 固定 1420 端口，忽略 src-tauri 监听
├── tailwind.config.js            # Tailwind 配置（关闭 preflight）
├── tsconfig.json / tsconfig.node.json
├── postcss.config.js
├── README.md / CLAUDE.md / DESIGN.md / FEATURES.md / CODE_REVIEW.md / 开发进度文档.md
├── entitlements.mac.plist        # macOS 权限声明
├── .github/workflows/
│   ├── build.yml                 # 四平台矩阵构建 + 打 tag 自动 Release
│   ├── build-windows.yml         # Windows(+macOS) 构建
│   └── README.md
├── src/                          # ===== 前端 (React/TS) =====
│   ├── main.tsx                  # ReactDOM 挂载
│   ├── App.tsx                   # 布局 + 页面切换 + 主题
│   ├── index.css                 # Tailwind 指令 + 全局样式
│   ├── components/
│   │   ├── layout/Sidebar.tsx
│   │   ├── device/DeviceList.tsx
│   │   └── transfer/FileDropZone.tsx, TransferList.tsx
│   ├── pages/HomePage.tsx, HistoryPage.tsx, SettingsPage.tsx
│   ├── services/tauriApi.ts      # 全部 invoke/listen 封装与 TS 类型
│   └── store/useDeviceStore.ts, useTransferStore.ts
└── src-tauri/                    # ===== 后端 (Rust) =====
    ├── Cargo.toml
    ├── build.rs                  # tauri_build::build()
    ├── tauri.conf.json           # 窗口、bundle、devUrl
    ├── capabilities/default.json # 权限：core:default + dialog:allow-open
    ├── icons/
    └── src/
        ├── main.rs               # 调用 app_lib::run()
        ├── lib.rs                # 应用入口与装配
        ├── core/                 # app_state / constants / error / config(空)
        ├── network/              # discovery / device / protocol / connection / quic_* / auth
        ├── transfer/             # manager / task / file_chunk / resume / rate_limiter
        ├── storage/mod.rs        # SQLite 全局实例与 DAO
        └── ipc/mod.rs            # #[tauri::command] 集合与事件发射辅助
```

> 注意：`src-tauri/src/core/config.rs` 是**空文件**，`core/mod.rs` 中也注释掉了 `pub use config::*`。

---

## 5. 快速开始

```bash
pnpm install              # 安装前端依赖（项目强制使用 pnpm）
pnpm dev                  # 仅起 Vite（http://localhost:1420），浏览器里没有 Tauri API
pnpm tauri dev            # 启动完整桌面应用（推荐）
pnpm tauri build          # 打包安装程序
```

环境要求：Node.js 18+/20、pnpm、Rust ≥ 1.77.2、Tauri CLI（`@tauri-apps/cli` 已在 devDependencies 中）。

完整说明见 [07 构建与运行](07-build-and-run.md)。

---

## 6. 核心概念

| 概念 | 代码位置 | 说明 |
|------|----------|------|
| `DeviceInfo` | `network/device.rs` | 运行时在线设备：UUID、名称、OS、IP、QUIC 端口、能力列表、`last_seen` |
| `KnownDevice` | `network/device.rs` | SQLite 中的已知设备（含离线），`capabilities` 为 JSON 字符串 |
| `DiscoveryPacket` | `network/protocol.rs` | UDP 发现报文（bincode 序列化），三种类型：`Announce` / `Response` / `Goodbye` |
| `ControlMessage` | `network/protocol.rs` | QUIC 控制消息信封：`message_type` + bincode `payload` |
| `DataPacket` | `network/protocol.rs` | 文件分片包：`ChunkMetadata` + `data` |
| `TransferTask` | `transfer/task.rs` | 传输任务（内存态，`Arc<tokio::Mutex>` 包裹存于 `DashMap`） |
| `TransferStatus` | `transfer/task.rs` | `Pending / Transferring / Paused / Completed / Failed / Canceled` |
| 设备四态 | `services/tauriApi.ts` | 前端 UI 概念：`scanning / connecting / online / offline`（后端只会推送 online/offline） |
| 能力 `Capability` | `network/device.rs` | `FolderTransfer` / `ResumeTransfer` / `MultiDeviceSend` / `P2PTransfer` |

---

## 7. 一分钟架构速览

```text
┌──────────────────────── React 前端 (WebView) ────────────────────────┐
│  App ─ Sidebar / HomePage / HistoryPage / SettingsPage               │
│    ├─ Zustand: useDeviceStore, useTransferStore                      │
│    └─ services/tauriApi.ts ── invoke(cmd) / listen(event) ──┐        │
└──────────────────────────────────────────────────────────────┼───────┘
                                                               │ Tauri IPC
┌──────────────────────────────────────────────────────────────▼───────┐
│                          Rust 后端 (Tokio)                           │
│  ipc/mod.rs  ← 18 个 #[tauri::command]                                │
│    ├─ transfer/manager.rs  ── 发送编排 / 分片 / 限速 / 续传            │
│    ├─ network/discovery.rs ── UDP 45678 广播与监听                    │
│    ├─ network/quic_server.rs ── QUIC 45679 监听 / 接收落盘            │
│    ├─ network/quic_client.rs ── 主动连接 / 传输请求                    │
│    ├─ network/connection.rs ── 连接池 + 自签名证书                     │
│    └─ storage/mod.rs ── SQLite (设置/历史/续传/设备)                   │
│  共享状态：AppState { devices: DashMap, transfer_tasks: DashMap, ... }│
└───────────────────────────────────────────────────────────────────────┘
                          │ UDP 45678        │ QUIC/UDP 45679
                          ▼                  ▼
                   其他设备上的同类应用实例
```

详见 [01 整体架构](01-architecture.md)。
