# 传书 Code Wiki

> 面向开发者的代码级文档。基于仓库当前源码（`master` 分支，含未提交改动）整理，描述**实际实现**，并与 README/FEATURES 等文档中的描述做了对照，差异见 [09-reference.md](./09-reference.md#5-与仓库既有文档的差异)。

## 项目是什么

「传书」是一个跨平台（Windows / macOS / Linux）的**局域网文件传输桌面应用**：

- 前端：React 19 + TypeScript + Ant Design 6 + Zustand 5 + Vite 7
- 后端：Rust + Tauri 2 + Tokio + QUIC（quinn 0.10）
- 设备发现：UDP 广播（端口 `45678`）+ 已知识备定向探测
- 文件传输：QUIC 双向流（端口 `45679`），分块 + CRC32 分片校验 + SHA-256 整文件校验 + 断点续传
- 持久化：SQLite（设置、传输历史、断点信息、已知设备）

## 文档目录

| 文档 | 内容 |
| --- | --- |
| [01-architecture.md](./01-architecture.md) | 整体架构、分层、模块依赖图、启动生命周期、并发模型 |
| [02-frontend.md](./02-frontend.md) | 前端页面/组件/状态管理/API 封装/事件订阅 |
| [03-core-and-ipc.md](./03-core-and-ipc.md) | `AppState`、错误类型、常量、全部 Tauri 命令与事件 |
| [04-network-layer.md](./04-network-layer.md) | 设备发现、设备模型、协议结构、认证握手、连接池、QUIC 服务端/客户端 |
| [05-transfer-pipeline.md](./05-transfer-pipeline.md) | 发送/接收管线、任务状态机、分块、限速、暂停/恢复/取消 |
| [06-storage-and-resume.md](./06-storage-and-resume.md) | SQLite 表结构、存储 API、断点续传位图与持久化 |
| [07-protocol-and-flows.md](./07-protocol-and-flows.md) | 线格式、消息类型、发现/认证/传输/续传时序图 |
| [08-build-and-run.md](./08-build-and-run.md) | 环境要求、开发/构建/打包命令、CI、端口、路径、排错 |
| [09-reference.md](./09-reference.md) | 常量表、类型映射、文件清单、已知问题与技术债、术语表 |

## 5 分钟跑起来

```bash
# 环境：Node 18+、pnpm、Rust 1.77.2+、Tauri 2 的平台依赖
pnpm install          # 安装前端依赖
pnpm tauri dev        # 启动桌面应用（会自动执行 pnpm dev）
pnpm build            # 仅类型检查 + 前端构建
```

Rust 侧检查（在 `src-tauri/` 下）：

```bash
cargo check
cargo fmt --check
cargo clippy --all-targets --all-features
```

## 仓库结构（源码部分）

```
chuanshu-app/
├── src/                         前端（React + TS）
│   ├── App.tsx                  根组件：主题 + 侧边栏 + 三个页面 + 接收请求弹窗
│   ├── main.tsx                 React 挂载入口
│   ├── components/
│   │   ├── device/DeviceList.tsx
│   │   ├── layout/Sidebar.tsx
│   │   └── transfer/{FileDropZone,TransferList,IncomingTransferRequestModal}.tsx
│   ├── pages/{HomePage,HistoryPage,SettingsPage}.tsx
│   ├── services/tauriApi.ts     IPC 调用 + 事件订阅 + 前端类型定义
│   └── store/{useDeviceStore,useTransferStore}.ts    Zustand
├── src-tauri/
│   ├── src/
│   │   ├── main.rs              二进制入口，调用 app_lib::run()
│   │   ├── lib.rs               Tauri Builder、插件、命令注册、服务启动与退出
│   │   ├── core/                AppState / 常量 / 错误（config.rs 为空）
│   │   ├── network/             discovery / device / protocol / auth / connection / quic_server / quic_client
│   │   ├── transfer/            manager / task / file_chunk / resume / rate_limiter
│   │   ├── storage/             SQLite 封装
│   │   └── ipc/                 Tauri 命令 + 事件发射 + 系统能力
│   ├── tauri.conf.json          窗口、打包、插件配置
│   └── capabilities/default.json Tauri 权限
├── .github/workflows/           多平台构建与 Release
├── DESIGN.md / FEATURES.md      设计系统与功能进度
└── docs/wiki/                   本文档
```

## 核心概念速览

| 概念 | 说明 |
| --- | --- |
| **设备发现（Discovery）** | 每 3 秒向各网卡广播地址发 `Announce`；收到后回复 `Response`；30 秒未收到心跳即判定离线；另有每 10 秒对数据库中的已知设备做定向探测，支持跨子网 |
| **认证（Auth）** | 每条 QUIC 连接的第一条双向流做 3 步握手（Hello → Response → Ack），校验对端 device_id 与 nonce 摘要；TLS 证书按发现包公布的 SHA-256 指纹固定 |
| **协议版本** | `1.1.0`（发现包、控制消息、认证统一校验；与 1.0.0 不兼容） |
| **连接池** | `ConnectionPool` 以 `device_id → PeerConnection` 缓存已认证连接，后续控制流/数据流复用同一连接 |
| **传输任务** | `TransferTask` 表示一次发送/接收，状态为 `Pending → Transferring ↔ Paused → Completed / Failed / Canceled` |
| **分块** | 按文件大小计算块大小（64KB / 1MB / 4MB / 8MB），每块带 CRC32；小文件（<100MB）额外计算整文件 SHA-256 |
| **并发** | 最多 5 个发送任务并行（`transfer_semaphore`），其余排队 |
| 断点续传 | 接收端用位图记录已收分片并持久化到 `resume_info` 表；发送端按接收端返回的 `missing_chunks` 只补发缺失块；元数据不一致时全量重传 |
| **前端四态设备状态** | `scanning` / `connecting` / `online` / `offline`，颜色见 [DESIGN.md](../../DESIGN.md) |

## 关键端口与路径

| 项 | 值 |
| --- | --- |
| UDP 发现端口 | `45678` |
| QUIC 传输端口 | `45679` |
| 协议版本 | `1.1.0` |
| 心跳间隔 | 3 s |
| 离线判定 | 30 s（25 s 用于“曾离线”判断） |
| QUIC 空闲超时 / 保活 | 60 s / 15 s |
| 最大并发发送 | 5 |
| 终态任务保留 | 1 小时 |
| 数据库 | `{app_data_dir}/chuanshu.db` |
| 默认下载目录 | `~/Downloads/传书`（Windows 为 `%USERPROFILE%\Downloads\传书`） |

## 阅读建议

- 想了解**一次传输怎么跑通**：先读 [07-protocol-and-flows.md](./07-protocol-and-flows.md)，再读 [05-transfer-pipeline.md](./05-transfer-pipeline.md)。
- 想改**发现/连接逻辑**：读 [04-network-layer.md](./04-network-layer.md)。
- 想加**设置项/历史/系统能力**：读 [03-core-and-ipc.md](./03-core-and-ipc.md) + [06-storage-and-resume.md](./06-storage-and-resume.md)。
- 想改**界面**：先读 `DESIGN.md`，再读 [02-frontend.md](./02-frontend.md)。
