# ADR 004: React Query 替代手动轮询

**状态**: 已接受 | **日期**: 2026-07-08

## 背景
前端使用手动 setInterval 轮询，存在竞态条件、无缓存、无自动刷新控制等问题。

## 决策
引入 TanStack React Query：useQuery + refetchInterval 实现自动轮询，内置缓存/去重/后台刷新。

## 后果
- 正面：消除竞态条件，缓存命中时瞬时渲染
- 负面：引入新依赖（~12KB gzip）
