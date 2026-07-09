# 贡献指南

## 开发环境

### 前置要求
- Node.js 20+, pnpm 9+, Rust 1.75+

### 搭建步骤
```bash
git clone https://github.com/jarvis/MultiAgents-Manager.git
cd MultiAgents-Manager
pnpm install
pnpm tauri:dev
```

## 代码规范
- Rust: rustfmt + cargo clippy 无警告
- TypeScript: Prettier + ESLint
- 提交信息: Conventional Commits (feat/fix/docs/refactor/test/chore)

## PR 流程
1. Fork 并创建分支: `git checkout -b feat/your-feature`
2. 确保测试通过: `cargo test && pnpm test`
3. 提交 PR，填写 PR 模板

## 本地测试
```bash
pnpm check          # format + lint + build
pnpm test           # vitest
cd src-tauri && cargo check && cargo test && cargo clippy -- -D warnings
```
