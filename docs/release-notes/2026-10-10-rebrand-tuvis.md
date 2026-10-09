# 品牌改名公告：MultiAgents-Manager (MAM) → Tuvis（兔维斯）

> 关联：issue #76（改名 Playbook）、`docs/brand/`（吉祥物与设计参数草稿）。
> **切换方式（2026-10 用户裁决）：一次性完成，不做分阶段过渡**——存量用户极少
> （每版本下载量个位数），接受旧客户端自动更新失效、手动重装。

## 新名字

**Tuvis**（中文名 **兔维斯**）—— 你的桌面多 Agent 指挥台。
「Tu」= 兔，「vis」致敬 Jarvis 的尾音：一只不知疲倦的机甲兔，替你盯着所有 Agent。

## 对用户的影响

| 项 | 变化 | 说明 |
|---|---|---|
| 应用名 / 窗口标题 / 托盘 | MultiAgents Manager → **Tuvis** | 纯显示层 |
| 安装包 / 更新包文件名 | `MultiAgents-Manager-*` → `Tuvis-*` | 从 release 页手动下载即可 |
| 仓库地址 | `.../MultiAgents-Manager` → `.../tuvis` | GitHub 旧地址永久重定向，收藏链接不死 |
| bundle identifier | `com.jarvis.multiagents-manager` → `com.jarvis.tuvis` | 见下方「一次性切换的代价」 |
| **数据目录 `~/.mam/`** | **不变** | 会话、账本、技能仓库、配置全部原地保留，`mam.db` 不迁移 |
| 自动更新（≤0.5.2 存量客户端） | **失效** | 清单发现与校验已全部指向新仓名，旧版应用内更新会失败——**请手动下载新版安装包** |
| Hook 信任 | 不受影响 | 状态 Hook 脚本内容与注册命令串零改动，无需重新 `/hooks` 信任 |

## 一次性切换的代价（如实申报）

- **旧客户端自动更新断链**：发现与校验均不再兼容旧仓名，≤0.5.2 升级必须手动。
- **手动重装而非覆盖升级**：Windows 的 NSIS 安装登记以 productName/identifier 派生，新旧目录不同——直接装新版会与旧版**双装并存**（两个托盘图标），建议先卸载旧版再装新版。
- **WebView2 本地存储重置一次**：identifier 变更使前端 localStorage 清零（主题偏好、移动端书签镜像等），`~/.mam/mam.db` 里的正式设置不受影响。

## 已知边界

- **内部代号 MAM 保留**：数据层路径（`~/.mam`、`mam.db`、`MAM_HOME`）、`mam-marker`/`mam-hook-listener` helper、进程标记串 `"MAM:hash"`、`mam-updater-progress` 事件名等不可见基础设施沿用旧代号，与显示品牌解耦；`docs/` 下的历史计划/发布记录为历史存档，不回溯改写。
- **宪法 `docs/MASTER-PLAN.md`** 的项目名修订按项目治理规则单独裁决。
- 发布前实机走一遍「卸载 0.5.2 → 安装新版」验收（首次启动数据完整性、托盘、hook 状态）。
