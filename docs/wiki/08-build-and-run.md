# 08 · 运行、构建与运维

## 1. 环境要求

| 组件 | 版本/要求 |
| --- | --- |
| Node.js | 18+（CI 使用 20） |
| 包管理器 | **pnpm**（禁止 npm；CI 使用 pnpm 9 / latest） |
| Rust | 1.77.2+（`Cargo.toml` 指定 `rust-version`） |
| Tauri CLI | 由 `@tauri-apps/cli` 2.10.x 提供 |
| Windows | MSVC 工具链 + WebView2 Runtime |
| macOS | Xcode Command Line Tools |
| Linux | GTK3、WebKitGTK 4.1、libsoup3、librsvg、appindicator 等（见 CI 安装列表） |

## 2. 常用命令

### 2.1 前端

```bash
pnpm install       # 安装依赖
pnpm dev           # Vite 开发服务器 http://localhost:1420（strictPort）
pnpm build         # tsc 类型检查 + vite build → dist/
pnpm preview       # 预览生产构建
```

### 2.2 桌面应用

```bash
pnpm tauri dev     # 开发模式（会自动执行 pnpm dev）
pnpm tauri build   # 打包当前平台的安装包
```

`src-tauri/tauri.conf.json` 中的 `beforeDevCommand` / `beforeBuildCommand` 已分别绑定 `pnpm dev` / `pnpm build`，因此直接运行 `tauri dev/build` 即可。

清理依赖时手动删除 `node_modules` 与 `pnpm-lock.yaml` 后重装（`package.json` 没有 `clean` script）。

### 2.3 Rust 检查

在 `src-tauri/` 下：

```bash
cargo check
cargo fmt --check
cargo clippy --all-targets --all-features
```

仓库**没有**前端单元测试（无 Jest/Vitest 配置）与集成测试；Rust 侧只有 `rate_limiter.rs` 内的 3 个 `#[cfg(test)]` 单元测试。

```bash
cd src-tauri && cargo test
```

### 2.4 构建产物

| 平台 | 路径/格式 |
| --- | --- |
| 前端 | `dist/`（`emptyOutDir: true`） |
| Windows | `src-tauri/target/release/bundle/msi/*.msi`、`bundle/nsis/*.exe` |
| macOS | `bundle/dmg/*.dmg`、`bundle/macos/*.app` |
| Linux | `bundle/deb/*.deb`、`bundle/appimage/*.AppImage` |

`bundle.targets = "all"`，图标需覆盖 `32x32.png`、`128x128.png`、`128x128@2x.png`、`icon.icns`、`icon.ico`。

## 3. Tauri 配置

[tauri.conf.json](../../src-tauri/tauri.conf.json) 关键项：

| 配置 | 值 | 说明 |
| --- | --- | --- |
| `productName` | `传书` | 安装包/应用名 |
| `identifier` | `com.chuanshu.app` | 决定 app_data 目录与 macOS LaunchAgent 的 Label |
| `build.frontendDist` | `../dist` | 生产前端资源 |
| `build.devUrl` | `http://localhost:1420` | 开发地址（与 Vite `strictPort` 一致） |
| `app.windows` | 1000×700，最小 800×600，可缩放，有装饰 | 单窗口 |
| `app.security.csp` | `null` | 未启用 CSP |
| `plugins.shell` | `{}` | shell 插件启用 |
| `plugins.fs` | `requireLiteralLeadingDot: false` | 允许非点号开头路径 |
| `bundle.macOS.entitlements` | `../entitlements.mac.plist` | 网络客户端/服务端、下载目录读写 |
| `bundle.windows.nsis` | 中英文、`icon.ico`、`header.bmp` | NSIS 安装器 |

[capabilities/default.json](../../src-tauri/capabilities/default.json) 只授予：

- `core:default`
- `dialog:allow-open`

前端目前只直接调用 `plugin-dialog` 的 `open`（选择下载目录），其余系统能力都通过自定义命令完成。

macOS [entitlements.mac.plist](../../entitlements.mac.plist) 声明了：

- `com.apple.security.network.client/server`
- `files.user-selected.read-write`、`files.downloads.read-write`
- `NSLocalNetworkUsageDescription`（局域网访问说明）
- `NSDownloadsFolderUsageDescription`（下载目录访问说明）

## 4. CI / 发布

`.github/workflows/`：

| 工作流 | 触发 | 任务 |
| --- | --- | --- |
| `build.yml` | push `main`、tag `v*`、PR 到 `main` | 矩阵构建：Windows x64、macOS Intel、macOS ARM、Linux x64；上传各自安装包；tag 时创建 draft Release |
| `build-windows.yml` | 同上 | Windows（先构建）+ macOS（仅 tag）；上传 MSI/NSIS/DMG；tag 时发布 |

注意事项（从配置读出）：

- Linux 构建依赖 `libwebkit2gtk-4.1-dev` 等系统包，CI 中已预装。
- macOS 未配置签名证书（`signingIdentity: null`），产物未签名。
- `build-windows.yml` 的 macOS job 使用默认 target，未显式 universal。
- 两个工作流都会在 tag 时创建 Release，注意避免重复发布。

## 5. 端口、网络与权限

| 项 | 值 | 需要放行 |
| --- | --- | --- |
| UDP 发现 | 45678 | Windows 防火墙入站 UDP；macOS 首次弹「允许本地网络」 |
| QUIC | 45679（UDP） | Windows 防火墙入站 UDP；路由器/交换机不得阻断 UDP |
| Vite 开发 | 1420（TCP） | 仅本机开发 |

部署检查：

1. 两台设备必须在同一网段，或把对方 IP 加入「已知设备」后等待 10s 探测（跨子网需要路由可达）。
2. 交换机若开启 **AP 隔离 / 客户端隔离**，UDP 广播无效，只能用 IP 手动发现。
3. VPN 或虚拟网卡可能让 `if_addrs` 选错广播地址；可临时禁用无关网卡验证。
4. Windows 首次运行需在防火墙中允许「传书」的专用/公用网络访问。
5. macOS 15+ 的本地网络隐私弹窗必须允许，否则 UDP 广播与 QUIC 都会被拒。

## 6. 运行时文件

| 内容 | 位置 |
| --- | --- |
| SQLite | `{app_data_dir}/chuanshu.db`；macOS 约 `~/Library/Application Support/com.chuanshu.app/`，Windows 约 `%APPDATA%\com.chuanshu.app\`，Linux 约 `~/.local/share/com.chuanshu.app/` |
| 默认下载目录 | `~/Downloads/传书`（可在设置中修改） |
| 接收临时文件 | 下载目录下的 `.{file_id}.tmp` |
| 日志 | debug 构建：`tauri-plugin-log`（Debug 级别）输出到控制台；release 未启用日志插件 |

## 7. 手动验收清单

网络与发现：

- [ ] 两台设备启动后 3–10 秒内互相出现在设备列表。
- [ ] 关闭对端应用，30 秒内状态变为离线（透明度 0.6）。
- [ ] 重新打开对端，状态恢复在线。
- [ ] 手动输入 IP 能在 3 秒内发现设备。
- [ ] 跨子网场景下，等待 10 秒探测周期后可见。

传输：

- [ ] 单文件发送、接收内容一致（对比 SHA-256）。
- [ ] 多文件 + 文件夹发送，目录结构保留。
- [ ] 接收端 `auto_accept=false` 时弹窗，接受后开始传输；60 秒不处理则自动拒绝。
- [ ] 传输中暂停 → 双方停止推进；恢复 → 继续。
- [ ] 传输中取消 → 双方任务置为已取消。
- [ ] 传输中断（杀进程/断网）后再次恢复，仅补传缺失分片。
- [ ] 历史记录出现对应条目，能打开位置、能重新发送。
- [ ] 限速滑块设为 10 时速度约为 10 MB/s（单位已统一为 MB/s）。

设置与系统：

- [ ] 修改下载路径后新任务保存到新路径。
- [ ] 开机自启开启/关闭在系统生效。
- [ ] 主题切换与跟随系统生效。
- [ ] 通知开关关闭后传输完成不再弹系统通知。

## 8. 排错手册

| 现象 | 可能原因 | 排查步骤 |
| --- | --- | --- |
| 设备列表为空 | 防火墙拦截 / AP 隔离 / 不同网段 / 端口占用 | 检查 45678 UDP；用 `nc -u -l 45678` 或 Wireshark 抓包；改用 IP 手动发现 |
| 设备显示离线但进程在跑 | 心跳未送达；30s 超时 | 检查广播地址是否选到虚拟网卡；查看日志 `Failed to broadcast discovery` |
| 点击发送提示「设备未找到」 | 内存 `devices` 已清理，但前端还显示已知设备 | 等待重新发现或手动 IP 发现；刷新列表 |
| QUIC 握手失败 / 一直超时 | 证书指纹不匹配、端口不通、对端未启动 | 先测 UDP 发现；确认对端已广播最新指纹；用 `lsof -i udp:45679` 确认监听 |
| 传输速度为 0 或极慢 | 限速设置、网络质量、`receive_window` | 把限速设为 0；确认版本为 1.1.0（接收窗口已调至 16 MiB） |
| 接收请求弹窗不出现 | `auto_accept=true`；或事件未订阅 | 检查设置；查看后端日志是否发出 `transfer-request` |
| 接受后对端 60 秒超时 | 前端未调用 accept/reject | 确认弹窗未被关闭且命令成功 |
| 传输完成但列表仍显示传输中 | 完成事件丢失 | 重新进入页面；事件由发送/接收端都会发出，不再依赖轮询 |
| 失败事件里任务 ID 为 undefined | 旧版本前端与后端字段不一致 | 升级到当前版本（前后端均使用 `task_id`） |
| 历史「重新发送」无反应 | 源文件已被移动/删除；对端不在线 | 检查 `source_path`；确认设备在线 |
| 续传后文件损坏 | 断点与源文件元数据不一致 | 当前版本会自动检测并全量重传；若仍有问题，删除下载目录下 `.tmp` 与 `resume_info` 记录后重传 |
| 设置页修改设备名无效 | 旧版本在 setup 中丢弃设置值 | 已修复：设备名在启动时从 SQLite 读取，重启后生效 |
| macOS 无法发现设备 | 本地网络权限被拒 | 系统设置 → 隐私与安全性 → 本地网络，重新允许 |
| Linux 构建失败 | 缺 WebKitGTK 依赖 | 按 CI 的 apt 列表安装 |

日志定位：

- 开发模式：终端输出 + WebView 控制台。
- 关键日志关键字：`Discovery service`、`QUIC server`、`Authentication`、`Transfer`、`checksum mismatch`、`Failed to`。
- 数据库排查：`sqlite3 chuanshu.db '.tables'`、`select * from known_devices;`、`select * from resume_info;`。

## 9. 扩展开发指南

### 9.1 新增一个 IPC 命令

1. 在 `src-tauri/src/ipc/mod.rs` 写 `#[tauri::command] pub async fn my_command(...) -> AppResult<T>`；
2. 在 `src-tauri/src/lib.rs` 的 `generate_handler![]` 中追加；
3. 在 `src/services/tauriApi.ts` 增加类型化包装（参数用 camelCase）；
4. 若涉及新权限，更新 `capabilities/default.json`；
5. 运行 `cargo check` + `pnpm build`。

### 9.2 新增一个事件

1. 在 `ipc/mod.rs` 定义事件名常量（或复用现有字符串）；
2. 用 `AppState::emit_async` / `AppHandle::emit` 发射；
3. 在 `tauriApi.ts` 增加 `onXxx` 包装；
4. 组件 `useEffect` 中订阅并返回 unlisten。

### 9.3 新增设置项

1. `Settings` 增加字段；
2. `Storage::save_settings` / `load_settings` 增加读写与默认值；
3. 前端 `Settings` 类型 + 设置页表单；
4. 使用处从 `settings` 读取。

### 9.4 修改协议

- 发现包/控制消息结构改动会使新旧版本不兼容；需同时提升 `PROTOCOL_VERSION` 并在 `handle_discovery_packet` 之外的控制层增加版本校验。
- `manager.rs` 与 `quic_server.rs` 各自有一份 `FileHeader`，改动需同步。

## 10. 相关文档

- 设计系统：[DESIGN.md](../../DESIGN.md)
- 功能进度：[FEATURES.md](../../FEATURES.md)
- 传输可靠性设计：[2026-09-24-transfer-reliability-design.md](../superpowers/specs/2026-09-24-transfer-reliability-design.md)
- 跨平台规范：[CLAUDE.md](../../CLAUDE.md)、[AGENTS.md](../../AGENTS.md)
- Windows 技术栈：[TECH-STACK-WINDOWS.md](../../TECH-STACK-WINDOWS.md)
