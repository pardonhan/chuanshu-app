# Design System — Chuanshu App

## Product Context
- **What this is:** 局域网文件传输工具，极简版
- **Who it's for:** 程序员在多台设备间传输代码、配置文件
- **Space/industry:** 跨平台文件传输 (Windows + macOS)
- **Project type:** 桌面应用 (Tauri + React)

## Aesthetic Direction
- **Direction:** Industrial/Utilitarian — 功能优先，数据密集，极简装饰
- **Decoration level:** Minimal — 排版和留白为主，无装饰元素
- **Mood:** 高效、冷静、可信赖。用户应该在 3 秒内发现设备并开始传输
- **Reference sites:** 无 — 极简工具类产品

## Typography
- **Display/Hero:** 系统字体栈 ( -apple-system, BlinkMacSystemFont, "Microsoft YaHei") — 保持原生体验
- **Body:** 系统字体栈 — 无额外字体加载，最快首屏
- **UI/Labels:** 系统字体栈
- **Data/Tables:** 系统字体栈 (tabular-nums 通过 CSS 实现)
- **Code:** 系统等宽字体
- **Scale:** 12px(辅助信息) / 14px(正文) / 16px(标题) / 20px(大标题)

## Color
- **Approach:** Restrained — 1 个强调色 + 中性色
- **Primary:** 未指定 — 使用 Ant Design 默认蓝
- **Neutrals:**
  - 文本：#333 (主要) / #666 (次要) / #999 (辅助) / #ccc (边框)
  - 背景：#fff (浅色) / #1a1a1a (深色)
- **Semantic:**
  - 扫描中：#9CA3AF (灰色)
  - 连接中：#F59E0B (黄色)
  - 在线：#10B981 (绿色)
  - 离线：#6B7280 (深灰色)
  - 成功：#10B981
  - 警告：#F59E0B
  - 错误：#EF4444
  - 信息：#3B82F6
- **Dark mode:** 未实现

## Spacing
- **Base unit:** 4px (Ant Design 基础单位)
- **Density:** Compact — 列表项 padding: 12px
- **Scale:** 4 / 8 / 12 / 16 / 24 / 32

## Layout
- **Approach:** Grid-disciplined — 严格左对齐，垂直层次清晰
- **Max content width:** 未指定 — 自适应窗口
- **Border radius:** 8px (卡片) / 4px (按钮)

## Motion
- **Approach:** Minimal-functional — 仅状态转换动画
- **Duration:** 200ms (状态转换)
- **Easing:** ease-in-out

## Device Status States (四态设备状态)

| State | Color | Icon | Label | Behavior |
|-------|-------|------|-------|----------|
| 扫描中 | #9CA3AF | SyncOutlined (旋转) | 扫描中 | 自动扫描网络设备 |
| 连接中 | #F59E0B | WifiOutlined | 连接中 | 正在建立 QUIC 连接 |
| 在线 | #10B981 | 无 | 在线 | 可点击选择 |
| 离线 | #6B7280 | 无 | 离线 | 不可点击，透明度 0.6 |

### Empty State (空状态)
- 显示"暂无设备记录" + "正在扫描网络设备..."动画
- 提供手动输入 IP 地址入口
- 说明文字确保用户理解需要同一网络

### Card States (设备卡片)
- 选中态：浅蓝色背景 #e6f7ff + 蓝色边框 #91d5ff
- 离线态：透明度 0.6，不可点击
- 在线态：正常显示，可点击

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-03-26 | 设备状态四态设计 | 完整反馈连接生命周期，用户明确知道发生了什么 |
| 2026-03-26 | 仅颜色编码无障碍方案 | 简化实现，状态标签文字已足够区分 |
| 2026-03-26 | 使用 Ant Design 默认颜色 | 快速实现，保持一致性 |
| 2026-03-26 | 无额外字体加载 | 最快首屏，原生体验 |
