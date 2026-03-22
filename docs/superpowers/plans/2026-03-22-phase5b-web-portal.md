# Phase 5b: Web 开发者门户 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建 React + TypeScript Web 前端，为运维人员和下游开发者提供可视化管理界面：项目管理、API 文档浏览、沙箱管理、补偿/死信管理、监控面板。

**Architecture:** 独立前端项目 `web/` 使用 Vite + React 18 + TypeScript + TailwindCSS。通过 Platform API 获取数据（已有完整的 REST 后端）。生产环境通过 Nginx 或 platform-api 的静态文件服务部署。

**Tech Stack:** React 18, TypeScript, Vite, TailwindCSS, React Router, fetch API

---

## File Structure

```
web/
├── package.json
├── tsconfig.json
├── vite.config.ts
├── index.html
├── src/
│   ├── main.tsx
│   ├── App.tsx
│   ├── api/
│   │   └── client.ts               # API 客户端封装
│   ├── pages/
│   │   ├── Dashboard.tsx            # 仪表盘（项目列表 + 统计）
│   │   ├── ProjectDetail.tsx        # 项目详情（路由、契约、沙箱）
│   │   ├── ApiDocs.tsx              # API 文档（嵌入 Swagger UI）
│   │   ├── SandboxManager.tsx       # 沙箱会话管理
│   │   ├── CompensationManager.tsx  # 死信/重推管理
│   │   └── Settings.tsx             # 系统设置
│   ├── components/
│   │   ├── Layout.tsx               # 侧边栏 + 顶栏布局
│   │   ├── ProjectCard.tsx
│   │   ├── RouteTable.tsx
│   │   ├── DeadLetterTable.tsx
│   │   └── SandboxSessionCard.tsx
│   └── styles/
│       └── index.css
```

---

### Task 1: Web 项目脚手架

- [ ] 使用 Vite 创建 React + TypeScript 项目
- [ ] 安装 TailwindCSS + React Router
- [ ] 创建基础 Layout 组件（侧边栏导航 + 主内容区）
- [ ] 创建 API 客户端封装（fetch wrapper with base URL）
- [ ] 配置 Vite proxy 指向 localhost:8080

Commit: `feat(web): initialize React + TypeScript frontend with Vite and TailwindCSS`

---

### Task 2: Dashboard + 项目管理页

- [ ] Dashboard 页：项目列表（卡片视图），显示名称、协议类型、路由数量
- [ ] 创建项目对话框
- [ ] 项目详情页：显示契约、路由列表、后端绑定信息
- [ ] 删除项目功能

Commit: `feat(web): add dashboard and project management pages`

---

### Task 3: API 文档页

- [ ] 嵌入 Swagger UI（iframe 指向 /api/v1/docs）
- [ ] Agent Prompt 查看器（从 /api/v1/docs/agent-prompt 获取 Markdown 并渲染）
- [ ] OpenAPI spec 下载按钮

Commit: `feat(web): add API documentation page with Swagger UI embed`

---

### Task 4: 沙箱管理页

- [ ] 沙箱会话列表（按项目筛选）
- [ ] 创建会话表单（选择 Mock/Replay/Proxy 模式）
- [ ] 会话详情：显示 base URL、模式、配置
- [ ] cURL 示例生成（含 X-Sandbox-Mode 和 X-Sandbox-Session 头）
- [ ] 删除会话

Commit: `feat(web): add sandbox session management page`

---

### Task 5: 补偿管理页

- [ ] 死信队列列表（分页、按路由筛选）
- [ ] 死信详情（请求 payload、错误信息、重试次数）
- [ ] 单条重推按钮
- [ ] 批量重推（勾选多条）
- [ ] 标记已处理

Commit: `feat(web): add compensation and dead letter management page`

---

### Task 6: 集成 + 部署配置

- [ ] platform-api 添加静态文件服务（serve web/dist/ 目录）
- [ ] Docker Compose 添加 web build step
- [ ] 生产环境 Dockerfile（multi-stage: node build → nginx serve）

Commit: `feat: add web portal build and deployment configuration`

---

## Summary

| Task | 页面 | 功能 |
|------|------|------|
| 1 | 脚手架 | Vite + React + Tailwind + Layout |
| 2 | Dashboard | 项目列表 + CRUD + 详情 |
| 3 | API 文档 | Swagger UI + Agent Prompt |
| 4 | 沙箱管理 | 会话 CRUD + cURL 示例 |
| 5 | 补偿管理 | 死信列表 + 重推 + 标记 |
| 6 | 部署 | 静态服务 + Docker |

**Phase 5b 验收标准：** Web 前端可通过浏览器访问所有管理功能，项目/沙箱/死信的 CRUD 操作均可通过界面完成，API 文档可在线浏览。
