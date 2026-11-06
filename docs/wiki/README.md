# 传书 Code Wiki

## 项目是什么

「传书」是一个跨平台的**局域网文件传输桌面应用**，同一局域网内的两台设备无需账号或服务器即可互发文件与文件夹。

| 层 | 技术 |
| --- | --- |
| 前端 | React 19 + TypeScript + Ant Design 6 + Zustand 5 + Vite 7 |
| 后端 | Rust + Tauri 2 + Tokio + QUIC（`quinn` 0.10） |
| 设备发现 | UDP 广播（`45678`）+ 已知设备定向探测 |
| 文件传输 | QUIC 双向流（`45679`），每 4 MB 一个数据分片，CRC32 + SHA-256 校验 |
| 持久化 | SQLite（`chuanshu.db`），保存设置、传输历史、已知设备 |

特点与边界：

- 传输协议为 v2：**一次传输 = 一个 QUIC 连接 = 一条双向流**，连接用完即关。
- 服务端使用自签名证书，客户端**显式跳过证书校验**；没有账号体系，也没有对端身份校验，仅适用于可信局域网。
- **不支持续传**：传输中断后需要重新发起整次传输。
- 无云端、无中继、无外部服务端。

## 文档目录

| 文档 | 内容 |
| --- | --- |
| [01-architecture.md](./01-architecture.md) | 系统上下文、分层结构、模块依赖、启动生命周期、并发模型 |
| [02-frontend.md](./02-frontend.md) | 页面、组件、Zustand 状态、`tauriApi` 封装、事件订阅与轮询 |
| [03-core-and-ipc.md](./03-core-and-ipc.md) | `AppState`、`AppError`、常量、15 个 Tauri 命令与 5 个事件 |
| [04-network-layer.md](./04-network-layer.md) | 设备发现、`DeviceInfo`、TLS 配置、QUIC 服务端/客户端 |
| [05-transfer-pipeline.md](./05-transfer-pipeline.md) | 发送/接收管线、任务状态机、进度、限速、暂停/取消/恢复 |
| [06-storage.md](./06-storage.md) | SQLite 表结构、设置/历史/已知设备 API，以及“没有续传”的说明 |
| [07-protocol-and-flows.md](./07-protocol-and-flows.md) | 线格式、帧类型、发现与传输时序 |
| [08-build-and-run.md](./08-build-and-run.md) | 环境要求、开发/构建命令、端口、路径、排错与 macOS 权限 |
| [09-reference.md](./09-reference.md) | 常量表、命令表、文件清单、已知限制、术语表 |

## 5 分钟跑起来

环境：Node 18+、pnpm、Rust 1.77.2+，以及 Tauri 2 对应平台的系统依赖。

```bash
pnpm install       # 安装前端依赖（用 npm 的话：npm install）
pnpm tauri dev     # 启动桌面应用（内部会执行 pnpm dev，Vite 固定 1420 端口）
pnpm build         # tsc 类型检查 + Vite 构建到 dist/
pnpm tauri build   # 生成当前平台的安装包
```

Rust 侧检查（在 `src-tauri/` 下）：

```bash
cargo check
cargo fmt --check
cargo clippy --all-targets --all-features
```

仓库没有前端测试脚本；改动需本地通过 `pnpm build` 与上述 Rust 检查，并手工验证发现、发送、暂停/恢复/取消与落盘。

## 仓库结构

```
chuanshu-app/
├── src/                              前端（React + TS）
│   ├── main.tsx                      React 挂载入口
│   ├── App.tsx                       主题、侧边栏、三个页面切换
│   ├── index.css                     Tailwind 指令 + 全局样式
│   ├── pages/
│   │   ├── HomePage.tsx              设备列表 + 拖拽区 + 传输列表
│   │   ├── HistoryPage.tsx           传输历史表格
│   │   └── SettingsPage.tsx          设置表单
│   ├── components/
│   │   ├── device/DeviceList.tsx     已知设备 + 在线状态 + IP 发现
│   │   ├── layout/Sidebar.tsx        三级菜单
│   │   └── transfer/
│   │       ├── FileDropZone.tsx      拖拽收集路径并发送
│   │       └── TransferList.tsx      任务进度与暂停/恢复/取消
│   ├── services/tauriApi.ts          invoke / listen 封装 + 前端类型
│   └── store/
│       ├── useDeviceStore.ts         设备状态
│       └── useTransferStore.ts       任务状态
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                   二进制入口，调用 app_lib::run()
│   │   ├── lib.rs                    Builder、插件、命令注册、服务启动/退出
│   │   ├── core/                     AppState / 常量 / 错误 / 路径工具
│   │   ├── network/                  discovery / device / protocol / tls / quic_server / quic_client
│   │   ├── transfer/                 manager / task / file_chunk / rate_limiter
│   │   ├── storage/                  SQLite 封装
│   │   └── ipc/                      Tauri 命令、事件、系统能力、通知
│   ├── capabilities/default.json     Tauri 权限（core:default、dialog:allow-open）
│   ├── tauri.conf.json               窗口、打包、插件配置
│   └── Cargo.toml
├── entitlements.mac.plist            macOS 网络与下载目录授权
├── DESIGN.md / FEATURES.md           设计系统与功能说明
└── docs/wiki/                        本目录
```

## 核心概念速览

| 概念 | 说明 |
| --- | --- |
| **发现（Discovery）** | 每 3 秒向各网卡广播 `Announce`；收到后单播 `Response`；30 秒未收到心跳移除设备；另有每 10 秒对数据库中的已知设备做定向探测 |
| **传输协议 v2** | `TRANSFER_PROTOCOL_VERSION = 2`；请求帧、响应帧加自定义标签帧，使用 bincode 序列化 |
| **单连接单流** | 发送端为每次传输新建 QUIC 连接，只开一条双向流；接收端只接受一条流，流结束即连接结束 |
| **TLS** | 服务端用 `rcgen` 生成自签名证书（CN=传书文件传输，SAN 含 `chuanshu.local`、`127.0.0.1`）；客户端用自定义 `NoCertificateVerification` 跳过校验，`server_name` 固定为 `chuanshu.local` |
| **传输任务** | `TransferTask`，状态 `Pending → Transferring ↔ Paused → Completed / Failed / Canceled`，保存在内存 `AppState.transfer_tasks` |
| **分片** | 固定 4 MB（`DEFAULT_CHUNK_SIZE`），每片带 CRC32；整文件结束帧带 SHA-256 |
| **自动接收** | `auto_accept` 默认 `true`；关闭后接收端直接拒绝，不会弹出确认框 |
| **限速** | 令牌桶；上传/下载限速各自独立，单位为字节/秒，前端以 MB/s 展示与输入 |
| **设备状态类型** | 类型上定义了 `scanning` / `connecting` / `online` / `offline` 四态，当前实现只会产生 `online` / `offline` |

## 关键端口与路径

| 项 | 值 |
| --- | --- |
| UDP 发现端口 | `45678` |
| QUIC 传输端口 | `45679` |
| 发现包协议版本 | `"1.0.0"`（字符串） |
| 传输协议版本 | `2`（`u16`） |
| 心跳间隔 | 3 s |
| 离线判定 / 清理周期 | 30 s（另有 25 s 的“曾离线”判断） |
| QUIC 空闲超时 / 保活 | 60 s / 10 s |
| QUIC 窗口 | 接收 16 MB、单流接收 8 MB、发送 16 MB |
| 分片大小 | 4 MB |
| 数据库 | `<app_data_dir>/chuanshu.db`，可用 `CHUANSHU_DATA_DIR` 覆盖目录 |
| 默认下载目录 | `<home>/Downloads/传书` |

## 阅读建议

- 想弄清**一次传输如何跑通**：先读 [07-protocol-and-flows.md](./07-protocol-and-flows.md)，再读 [05-transfer-pipeline.md](./05-transfer-pipeline.md)。
- 想改**发现或连接逻辑**：读 [04-network-layer.md](./04-network-layer.md)。
- 想加**设置项、历史记录或系统能力**：读 [03-core-and-ipc.md](./03-core-and-ipc.md) 与 [06-storage.md](./06-storage.md)。
- 想改**界面**：先读 `DESIGN.md`，再读 [02-frontend.md](./02-frontend.md)。
- 想查**常量、命令、文件位置**：直接看 [09-reference.md](./09-reference.md)。
