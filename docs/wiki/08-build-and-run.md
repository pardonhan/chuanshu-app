# 08 · 构建与运行

## 1. 环境要求

| 项 | 要求 |
| --- | --- |
| Node.js | 18 及以上 |
| 包管理器 | `pnpm`（仓库脚本按 pnpm 编写，不要用 npm） |
| Rust | 1.77.2 及以上（`src-tauri/Cargo.toml` 的 `rust-version`） |
| Tauri | 2.x（CLI 由 `@tauri-apps/cli` 提供） |
| Windows | MSVC 构建工具、WebView2 Runtime |
| macOS | Xcode Command Line Tools |
| Linux | Tauri 2 官方依赖（`webkit2gtk-4.1`、`libxdo`、`librsvg2`、`libayatana-appindicator3` 等） |

前端依赖版本见 `package.json`：React 19、Ant Design 6、Zustand 5、Vite 7、TypeScript 5.9。

## 2. 安装与开发

```bash
pnpm install
pnpm tauri dev
```

- `pnpm dev` 单独启动 Vite，固定端口 `1420`（`strictPort: true`），并忽略 `src-tauri/**` 的文件监听。
- `pnpm tauri dev` 会按 `tauri.conf.json` 的 `beforeDevCommand` 自动执行 `pnpm dev`，再启动桌面窗口。
- 开发模式下 Rust 侧注册 `tauri-plugin-log`，日志级别 `Debug`；release 构建不注册该插件。

前端只有 Vite 构建，没有路由、没有 SSR、没有额外的 dev server 代理。

## 3. 构建与检查

```bash
pnpm build          # tsc 类型检查 + vite build，输出 dist/
pnpm preview        # 本地预览 dist/
pnpm tauri build    # 生成当前平台安装包
```

Rust 检查（在 `src-tauri/` 下执行）：

```bash
cargo check
cargo fmt --check
cargo clippy --all-targets --all-features
```

产物位置：

- 前端：`dist/`
- Rust 编译中间物：`src-tauri/target/`
- 安装包：`src-tauri/target/release/bundle/`
  - Windows：NSIS 安装包（图标 `icons/icon.ico`，语言 `SimpChinese` / `English`）
  - macOS：`.app` / `.dmg`（`signingIdentity` 为 `null`，不签名）
  - Linux：按 `targets: "all"` 生成当前环境可用的包

所有构建与检查都在本地完成，没有额外的流水线配置。

## 4. 端口与网络

| 用途 | 协议/端口 | 方向 |
| --- | --- | --- |
| 设备发现 | UDP `45678` | 双向；接收广播与单播，向广播地址与已知 IP 发送 |
| 文件传输 | UDP `45679` | 双向；QUIC |

排查前先确认：

- 两端在同一广播域，或已通过“输入 IP 发现”把对方写入已知设备。
- 系统防火墙允许上述 UDP 端口；Windows 首次运行通常弹窗询问，需勾选专用网络。
- 路由器未开启 AP 隔离/客户端隔离。
- VPN、虚拟网卡、Docker 网桥可能改变广播地址枚举结果。

## 5. 数据目录与默认路径

| 项 | 位置 |
| --- | --- |
| 数据目录 | `CHUANSHU_DATA_DIR`，否则 Tauri `app_data_dir()` |
| 数据库 | `<数据目录>/chuanshu.db` |
| 默认下载目录 | `<home>/Downloads/传书` |
| 设置中的下载目录 | 支持绝对路径、`~/...`、`~\...`；空串回退默认目录 |

`<home>` 的解析顺序是 `USERPROFILE` → `HOME` → `"."`。接收文件前会 `create_dir_all` 下载目录，因此目录不存在不会导致失败，但父路径无写权限会失败。

## 6. 环境变量

| 变量 | 作用 |
| --- | --- |
| `CHUANSHU_DATA_DIR` | 覆盖数据目录，主要用于同机双开测试 |
| `VITE_*` / `TAURI_*` | Vite `envPrefix` 声明的前端环境变量前缀 |

同机双开示例（PowerShell）：

```powershell
$env:CHUANSHU_DATA_DIR = "D:\tmp\chuanshu-b"
pnpm tauri dev
```

第二个实例会使用独立的数据库、设备 ID 与设置。注意两端仍会争用 `45678` / `45679`：发现 socket 在 macOS 上额外启用了 `SO_REUSEPORT`，Windows/Linux 只设置 `SO_REUSEADDR`；QUIC 服务端第二次绑定同一端口会失败并记录 `QUIC service failed to start`。

## 7. 常见问题

| 现象 | 排查方向 |
| --- | --- |
| 设备列表一直为空 | 检查是否同一网段、防火墙、AP 隔离；用 IP 输入框定向发现 |
| 设备显示在线但发送失败 | 对端 QUIC 服务未启动（端口被占用或启动报错），或对端已退出但心跳尚未超时 |
| 发送被立即拒绝 | 对端关闭了“自动接收文件”，或两端传输协议版本不一致 |
| 报“Storage not initialized” | 数据目录不可写、`CHUANSHU_DATA_DIR` 指向非法路径，查看启动日志 |
| 收到文件但提示校验失败 | 传输过程中源文件被修改，或网络/磁盘异常；重新发送 |
| 下载目录写入失败 | 目录无权限、路径非法（Windows 盘符/保留名） |
| 修改限速没有立即生效 | 限速在每次传输开始时读取，只影响新任务 |
| 前端显示 100 MB/s 但后端值很大 | 前端单位是 MB/s，后端是 B/s，换算只在 `tauriApi.ts` 做 |
| 设置页快捷键无效 | 页面只展示提示文本，代码未绑定快捷键 |
| macOS 收不到设备 | 检查“本地网络”权限与防火墙，见下一节 |
| 通知不出现 | 检查设置中的“启用系统通知”，以及系统通知权限 |

启动日志是首选证据：debug 构建会输出数据库路径、发现端口、QUIC 端口与连接/传输错误。

## 8. macOS 权限与签名

`entitlements.mac.plist` 声明了：

| 键 | 用途 |
| --- | --- |
| `com.apple.security.network.client` | 发起 QUIC 连接、发送 UDP |
| `com.apple.security.network.server` | 监听 QUIC 端口、接收 UDP |
| `com.apple.security.files.user-selected.read-write` | 选择下载目录 |
| `com.apple.security.files.downloads.read-write` | 写入 `~/Downloads/传书` |
| `NSLocalNetworkUsageDescription` | 本地网络访问说明 |
| `NSDownloadsFolderUsageDescription` | 下载文件夹访问说明 |

注意事项：

- 首次运行时 macOS 会弹出“允许访问本地网络”，拒绝后 UDP 发现与 QUIC 传输都会失败；需到“系统设置 → 隐私与安全性 → 本地网络”重新开启。
- 打包配置的 `signingIdentity` 为 `null`，产物未签名/未公证；分发给其他机器时 Gatekeeper 可能拦截，需要手动“仍要打开”或自行签名。
- 系统防火墙若拦截入站连接，需在“防火墙选项”中允许该应用。
- 修改权限后通常需要重启应用才能重新触发授权提示。
