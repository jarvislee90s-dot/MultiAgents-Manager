// 遗留 codex 技能链接迁移 Hook（spec §4.3 触发 + §6 关闭语义）
// 注意：后端 auto_import_extensions / sync_imported_skill_links 在独立线程后台执行
// （lib.rs "不阻塞启动"），与前端并发——mount 后 detect 的结果只是启动瞬间的快照，
// 迁移命令内部会逐条重验谓词并兼容"后台补链已抢先建好同 target 链接"的竞态
// （migration.rs 等效迁移豁免），此处无需额外就绪信号
import { useCallback, useEffect, useState } from "react";
import {
  detectLegacyAgentsLinks,
  migrateLegacyAgentsLinks,
  type MigrationItemReport,
  type MigrationMode,
} from "@/lib/api/skillMigration";

// Hook 对外状态（对话框组件按此渲染，home 挂载时二者并联）
export interface LegacySkillMigrationState {
  // 检测命中的遗留技能名清单（空 = 无遗留）
  skills: string[];
  // 对话框是否可见（detect 命中 ≥1 条时置真）
  visible: boolean;
  // 迁移/保留执行中（按钮 disabled 防重复提交）
  running: boolean;
  // 执行后的逐条报告；null = 尚未执行（选择视图），非 null = 结果视图
  reports: MigrationItemReport[] | null;
  // 整体调用失败信息（区别于逐条报告的 error 项；用于对话框内提示）
  runError: string | null;
  run: (mode: MigrationMode) => Promise<void>;
  close: () => void;
}

// 本次应用运行内已检测过的标记（模块级，SPA 内离开再回首页不重弹；
// 「关闭 = 下次启动再提示」的语义以一次应用运行为界，见 review Minor）
let detectedThisRun = false;

// 仅供测试：vitest 同文件多用例共享模块实例，beforeEach 需重置一次性检测标记
export function __resetLegacyMigrationDetectionForTest() {
  detectedThisRun = false;
}

export function useLegacySkillMigration(): LegacySkillMigrationState {
  const [skills, setSkills] = useState<string[]>([]);
  const [visible, setVisible] = useState(false);
  const [running, setRunning] = useState(false);
  const [reports, setReports] = useState<MigrationItemReport[] | null>(null);
  const [runError, setRunError] = useState<string | null>(null);

  // mount 后检测一次；失败静默（console.error + 视为无遗留，不打扰用户）
  useEffect(() => {
    if (detectedThisRun) return;
    let cancelled = false;
    detectLegacyAgentsLinks()
      .then((names) => {
        // flag 在异步返回后置位（review N-2）：StrictMode（main.tsx）双挂载下
        // effect#1 会被 cleanup 取消，若在 effect 内同步置位，effect#2 会被 flag
        // 拦住、对话框在 dev 永不出现；detect 失败同样不烧 flag，下次挂载可重试
        detectedThisRun = true;
        if (cancelled || names.length === 0) return;
        setSkills(names);
        setVisible(true);
      })
      .catch((error) => {
        console.error("detect_legacy_agents_links failed:", error);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // run：执行迁移/保留并暂存逐条报告（对话框切换为结果视图）
  const run = useCallback(async (mode: MigrationMode) => {
    setRunning(true);
    setRunError(null);
    try {
      const result = await migrateLegacyAgentsLinks(mode);
      setReports(result);
    } catch (error) {
      // 整体调用失败（区别于逐条 error）：记录详情供对话框展示
      console.error("migrate_legacy_agents_links failed:", error);
      setRunError(error instanceof Error ? error.message : String(error));
    } finally {
      setRunning(false);
    }
  }, []);

  // close：仅关对话框、零副作用（spec §6：不做任何变更，下次启动谓词仍命中会再提示
  // ——自熄灭机制靠识别谓词本身，无需持久化「不再提示」开关）
  const close = useCallback(() => setVisible(false), []);

  return { skills, visible, running, reports, runError, run, close };
}
