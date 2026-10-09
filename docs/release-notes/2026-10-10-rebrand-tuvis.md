# 品牌改名公告：MultiAgents-Manager (MAM) → Tuvis（兔维斯）

> 发布形式：随下一个版本（v0.5.3+）的 Release 说明保留旧名一次，保搜索连续性。
> 关联：issue #76（改名 Playbook）、`docs/brand/`（吉祥物与设计参数草稿）。

## 新名字

**Tuvis**（中文名 **兔维斯**）—— 你的桌面多 Agent 指挥台。
「Tu」= 兔，「vis」致敬 Jarvis 的尾音：一只不知疲倦的机甲兔，替你盯着所有 Agent。

## 对用户的影响

| 项 | 变化 | 说明 |
|---|---|---|
| 应用名 / 窗口标题 / 托盘 | MultiAgents Manager → **Tuvis** | 纯显示层 |
| 安装包 / 更新包文件名 | `MultiAgents-Manager-*` → `Tuvis-*` | 通过应用内自动更新升级的用户**无感**（latest.json 自动指向新文件名） |
| 仓库地址 | `.../MultiAgents-Manager` → `.../tuvis` | GitHub 旧地址永久重定向，已收藏链接不会失效 |
| **数据目录 `~/.mam/`** | **不变** | 会话、账本、技能仓库、托盘配置全部原地保留，`mam.db` 不迁移 |
| 自动更新 | **发布与改名有时序约束** | 见下方「发布顺序（硬约束）」：先发版、后改名 |
| Hook 信任 | 不受影响 | 状态 Hook 脚本内容与注册命令串零改动，无需重新 `/hooks` 信任 |

## 发布顺序（硬约束，防存量用户断更新）

本版代码的更新发现与清单校验均已做新旧仓名双兼容，但 **≤0.5.2 的存量客户端**编译死的校验只认旧仓名前缀——所以顺序必须是：

1. **先以旧仓名仓库发布本版**（旧仓发布对新旧客户端都工作）；
2. 观察升级率：存量客户端只有升到本版，才拿到双前缀校验能力；
3. 升级率收敛后，再执行 `gh repo rename tuvis` + 本地 `git remote set-url`；
4. 改名后仍未升级的残留客户端会因清单前缀校验失败而无法自动更新 → 需手动下载安装包（改名公告置顶提示）。

## 已知边界（如实申报）

- **bundle identifier 暂未改**（仍为 `com.jarvis.multiagents-manager`）：改 identifier 会重置存量用户的 WebView2 本地存储，留到上架商店前专项决策（issue #76 唯一硬时点）。
- **升级验收**：本版发布前实机走一遍 Windows 0.5.2 → 本版的应用内升级（NSIS 安装登记以 productName 派生，改名后需确认是覆盖升级而非双装）；旧版遗留在 `%TEMP%` 的 `multi-agents-manager-*-updater-*` 临时目录不再被新版的清理逻辑扫描（一次性孤儿目录，无碍）。
- **内部代号 MAM 保留**：数据层路径（`~/.mam`、`mam.db`、`MAM_HOME`）、`mam-marker`/`mam-hook-listener` helper、进程标记串 `"MAM:hash"` 等不可见基础设施沿用旧代号，与显示品牌解耦；`docs/` 下的历史计划/发布记录为历史存档，不回溯改写。
- 宪法 `docs/MASTER-PLAN.md` 的项目名修订需按项目治理规则单独裁决。
