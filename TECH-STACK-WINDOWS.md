# 传书 (Chuanshu App) - Windows 平台技术栈

> 生成时间: 2026-05-09

## 应用框架

| 层级 | 技术 | 版本 |
|------|------|------|
| 桌面框架 | **Tauri 2.x** | ^2.10.1 |
| Rust Edition | 2021 (Rust 1.77.2+) | - |

## 前端技术栈

| 技术 | 版本 | 用途 |
|------|------|------|
| **React** | ^19.2.4 | UI 框架 |
| **TypeScript** | ^5.9.3 | 类型安全 |
| **Vite** | ^7.3.1 | 构建工具 |
| **Ant Design** | ^6.3.3 | UI 组件库 |
| **Tailwind CSS** | ^4.2.2 | 原子化样式 |
| **Zustand** | ^5.0.12 | 状态管理 |
| **Day.js** | ^1.11.20 | 日期处理 |
| **clsx** | ^2.1.1 | 条件类名 |
| **filesize** | ^11.0.13 | 文件大小格式化 |

## 后端 (Rust) 技术栈

### 核心依赖

| 类别 | 库 | 用途 |
|------|----|------|
| **异步运行时** | tokio (full) + tokio-util + futures | 异步 I/O 调度 |
| **QUIC 协议** | quinn + rustls + rustls-pemfile | 加密的 UDP 传输 |
| **Socket** | socket2 + if-addrs | 网络接口发现 |
| **文件处理** | memmap2 + walkdir + filetime | 内存映射、目录遍历、时间戳 |
| **数据校验** | xxhash-rust + sha2 + crc32fast | 快速哈希/校验 |
| **序列化** | bincode + serde + serde_json | 二进制/JSON 序列化 |
| **数据库** | rusqlite (bundled + chrono) | SQLite 嵌入式存储 |
| **配置存储** | confy | 配置文件管理 |
| **日志追踪** | log + tracing + tracing-subscriber | 结构化日志 |
| **工具库** | uuid + chrono + dashmap + bitvec + rcgen | 通用工具/并发/证书生成 |
| **错误处理** | thiserror + anyhow | 错误类型定义 |
| **Tauri 插件** | tauri-plugin-fs/notification/shell/dialog | 系统能力扩展 |

## Windows 打包配置

| 项目 | 配置 |
|------|------|
| 安装包类型 | **NSIS** 安装程序 |
| 安装图标 | `icons/icon.ico` |
| Header 图片 | `icons/header.bmp` |
| 安装语言 | 简体中文、English |
| 窗口尺寸 | 1000x700 (最小 800x600) |
| 窗口特性 | 不透明, 带系统装饰边框 |

## Windows 平台关键特性

- **QUIC 协议传输**：基于 `quinn` + `rustls` 的加密 UDP 传输，Windows 上通过 `socket2` 处理平台特定的 socket 配置
- **SQLite 捆绑编译**：`rusqlite` 使用 `bundled` 特性，Windows 上无需额外安装 SQLite 依赖
- **条件编译适配**：使用 `#[cfg(target_os = "windows")]` 处理平台差异（防火墙规则、权限模型等）
- **网络发现**：`if-addrs` 用于局域网 IP 扫描和设备发现
- **包管理器**：pnpm（前端依赖管理）

## 构建命令

```bash
pnpm install          # 安装前端依赖
pnpm tauri build      # 构建 Windows NSIS 安装包
```
