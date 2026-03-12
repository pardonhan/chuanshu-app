# Chuanshu App 项目规范

## 包管理器

**必须使用 pnpm**，禁止直接使用 npm。

### 安装依赖
```bash
pnpm install
```

### 添加依赖
```bash
pnpm add <package-name>
pnpm add -D <package-name>  # 开发依赖
```

### 运行脚本
```bash
pnpm dev          # 启动 Vite 开发服务器
pnpm build        # 构建生产版本
pnpm preview      # 预览生产构建
pnpm tauri dev    # 启动 Tauri 开发环境
pnpm tauri build  # 构建 Tauri 应用
```

### 清理依赖
```bash
pnpm clean        # 清理 node_modules 和 pnpm-lock.yaml
pnpm install      # 重新安装
```

## 开发规范

### 代码风格
- 前端使用 TypeScript
- 遵循 ESLint 配置
- 使用 Prettier 格式化代码

### Git 提交
- 使用语义化提交信息 (feat/fix/docs/style/refactor/perf/test/chore)
- 提交前确保代码通过编译

### 分支管理
- `master` - 主分支，保持可构建状态

## 跨平台开发规范

本项目支持 **Windows** 和 **macOS**，开发时需注意以下适配事项：

### Rust 后端适配

#### 条件编译
使用 `#[cfg(target_os = "...")]` 处理平台差异：

```rust
#[cfg(target_os = "windows")]
// Windows 特定代码

#[cfg(target_os = "macos")]
// macOS 特定代码
```

#### 文件系统
- **路径分隔符**：使用 `std::path::PathBuf`，避免硬编码 `/` 或 `\`
- **权限处理**：Windows 和 macOS 文件权限模型不同，需分别处理
- **特殊目录**：使用 `dirs` crate 获取跨平台的用户目录

#### 网络
- **防火墙**：Windows 可能需要额外配置防火墙规则
- **广播地址**：确保 UDP 广播在所有平台上正常工作

### 前端适配

#### 路径处理
- 使用 Tauri API 获取路径，避免硬编码
- 文件路径显示时考虑不同系统的路径格式

#### UI/UX
- 窗口尺寸和 DPI 在不同系统上可能不同
- 字体渲染差异，确保通用字体栈

### 构建配置

#### tauri.conf.json
- `bundle.windows` - Windows NSIS 安装包配置
- `bundle.macos` - macOS app/dmg 配置
- 确保图标资源包含所有平台所需尺寸

### 测试要求
- 代码提交前确保在两个平台上都能编译通过
- 涉及系统调用的代码必须在两个平台上测试
