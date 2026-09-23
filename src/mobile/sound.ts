// 移动端提示音（M3 Task 6 提醒三件套之二）——自有合成音 + 总开关
//
// **不复用桌面 src/lib/audio.ts**：其 12 个音效资产在 public/ 下（约 9.5MB），
// 未随移动产物分发（vite.config.mobile.ts 的 publicDir=public-mobile），且那套
// 配置/试听 UI 属桌面域——移动 bundle 引它必然拿不到音频文件而静默失败。
// 口径（何时响）与桌面端统一，音源（怎么响）各自本地渲染。
//
// 音型（2026-09-19 用户裁决）：两音上行 E5→A5「叮-咚」。原实现是单正弦 880Hz
// 硬起硬停（osc.start() → stop(t+0.15)，中间无增益包络）——能量瞬间跳变，
// 听感即「监护仪滴滴声」。**包络淡入淡出是去电子感的关键**，比换音符更重要。

/** 持久化 key：与桌面 mam-sound-config（12 音效 + 按工具覆盖体系）刻意分叉——
 *  移动端只有开/关两态，引入桌面体系会平添概念负担且语义打架 */
const KEY = "mam-mobile-sound";

/** 音型：两音上行（E5 → A5）。at = 相对起播的触发时刻（秒），dur = 单音包络总长 */
const NOTES: ReadonlyArray<{ freq: number; at: number; dur: number }> = [
  { freq: 659.25, at: 0, dur: 0.28 }, // E5
  { freq: 880.0, at: 0.09, dur: 0.34 }, // A5
];
/** 单音峰值增益：低增益，手机默认音量下不刺耳（沿用旧实现的 0.08 量级） */
const PEAK_GAIN = 0.09;
/** 起音时长（秒）：快速淡入，避免咔哒声 */
const ATTACK_S = 0.012;

/** 声音是否开启：localStorage 不可用（隐私模式 Safari 抛 SecurityError）时按开启处理 */
export function getSoundEnabled(): boolean {
  try {
    return localStorage.getItem(KEY) !== "off";
  } catch {
    return true; // 读失败按默认开启（提示音是功能项，不因存储不可用而静默失去）
  }
}

/** 翻转声音开关：写 localStorage 持久化 + 返回新值。写失败仍返回已翻转值——
 *  本次会话内可继续往返切换，仅刷新后回落默认（与 theme.ts toggleTheme 同款容错） */
export function toggleSoundEnabled(): boolean {
  const next = !getSoundEnabled();
  try {
    localStorage.setItem(KEY, next ? "on" : "off");
  } catch {
    // 写失败：持久化放弃，返回值已翻转
  }
  return next;
}

// 懒建单例 AudioContext：Safari 对每页 AudioContext 数量有硬上限（约 6 个），
// 每次提醒新建会在数次提醒后耗尽配额、之后全部静默失败
let chimeCtx: AudioContext | null = null;

/**
 * 播放完成提示音（仅「任务完成→绿」时由调用方触发，见 Board.handleTransition）。
 * 整体 try/catch：提示音是锦上添花，任何失败（无该 API / 自动播放策略挂起）
 * 都不得中断提醒链路（横幅与振动仍在）。
 */
export function playCompletionChime(): void {
  try {
    if (!chimeCtx) {
      const Ctx = window.AudioContext;
      if (!Ctx) return; // 无 Web Audio 的环境（含 jsdom / 老浏览器）：跳过
      chimeCtx = new Ctx();
    }
    const ctx = chimeCtx;
    // 自动播放策略：无用户手势时 context 处于 suspended，resume 可能被拒——
    // 拒绝即本次无声，用户下次触摸页面后的提醒会正常出声，不额外处理
    if (ctx.state === "suspended") ctx.resume().catch(() => {});
    for (const note of NOTES) {
      const t0 = ctx.currentTime + note.at;
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.value = note.freq;
      // 包络：0 → 峰值（线性起音）→ 近零（指数衰减）。exponentialRamp 目标不可为 0，
      // 用 0.0001 近似静音；指数衰减的听感远比线性自然（无「一刀切」感）
      gain.gain.setValueAtTime(0, t0);
      gain.gain.linearRampToValueAtTime(PEAK_GAIN, t0 + ATTACK_S);
      gain.gain.exponentialRampToValueAtTime(0.0001, t0 + note.dur);
      osc.connect(gain).connect(ctx.destination);
      osc.start(t0);
      osc.stop(t0 + note.dur);
    }
  } catch {
    /* 静默降级：无提示音，其余提醒通道不受影响 */
  }
}
