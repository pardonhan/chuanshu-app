# Tauri 自动构建工作流

本工作流会在以下情况触发：
- 推送到 `main` 分支
- 创建 `v*` 格式的标签（如 v1.0.0）

## 使用方法

### 1. 推送代码到 GitHub

```bash
git init
git add .
git commit -m "Initial commit"
git branch -M main
git remote add origin https://github.com/你的用户名/你的仓库名.git
git push -u origin main
```

### 2. 创建 Release 版本

当你想要发布新版本时：

```bash
git tag v0.1.0
git push origin v0.1.0
```

### 3. 查看构建进度

访问：`https://github.com/你的用户名/你的仓库名/actions`

### 4. 下载安装包

- 构建完成后，在 Actions 页面点击对应的工作流
- 在页面底部的 "Artifacts" 部分下载对应平台的安装包
- 如果创建了 tag，会自动发布到 Releases 页面

## 输出文件

| 平台 | 文件格式 | 说明 |
|------|----------|------|
| Windows | `.msi`, `.exe` | MSI 安装程序和 NSIS 安装程序 |
| macOS Intel | `.dmg`, `.app` | Intel Mac 安装包 |
| macOS ARM | `.dmg`, `.app` | Apple Silicon Mac 安装包 |
| Linux | `.deb`, `.AppImage` | Debian 包和 AppImage |

## 注意事项

1. 第一次运行可能需要较长时间下载依赖
2. macOS 构建无法签名（需要开发者证书）
3. Linux 构建需要较多依赖库
