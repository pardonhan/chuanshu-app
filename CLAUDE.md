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
