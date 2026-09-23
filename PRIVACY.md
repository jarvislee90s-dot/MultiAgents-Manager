# 隐私声明（Privacy Statement）

## 数据原则

- **不内置遥测**：本应用不收集、不上传用户数据，不含任何统计或广告 SDK。
- **本地存储**：所有数据（会话索引、技能/预设/插件、SQLite 数据库）仅存于本机 `~/.mam/` 目录。
- **读取范围**：仅在用户操作对应功能时读取受支持 Agent 工具的本地配置与会话文件，用于展示与管理。
- **更新检查**：检查新版本时会访问 GitHub Releases 接口（仅获取版本信息，不含用户数据）。

## 远程访问功能（M2+）

看板与消息注入的数据经由**用户自行配置**的通道（局域网直连，或用户自有隧道/域名，如 Cloudflare Tunnel）在设备间传输。本项目当前不运营任何官方服务器，不留存任何传输数据。未来若提供官方托管中继服务，将随该服务另行发布专门的隐私政策与服务条款。

## English Summary

MultiAgents-Manager is local-first: no telemetry, no analytics or ad SDKs; all data stays on your machine (`~/.mam/`). Supported agents' local files are read only to power management features. Update checks contact GitHub Releases only. Remote-access features (M2+) travel over channels you configure yourself; no official servers are operated today.
