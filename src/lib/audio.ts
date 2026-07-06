// Web Audio API 提示音 — 支持用户配置频率

let audioCtx: AudioContext | null = null;

function getAudioContext(): AudioContext | null {
  if (typeof window === "undefined") return null;
  if (!audioCtx) {
    audioCtx = new AudioContext();
  }
  if (audioCtx.state === "suspended") {
    audioCtx.resume();
  }
  return audioCtx;
}

/** 播放单音 */
function playTone(frequency: number, duration: number, delay: number = 0) {
  const ctx = getAudioContext();
  if (!ctx) return;

  const oscillator = ctx.createOscillator();
  const gain = ctx.createGain();

  oscillator.connect(gain);
  gain.connect(ctx.destination);

  const startTime = ctx.currentTime + delay;
  oscillator.frequency.value = frequency;
  oscillator.type = "sine";

  gain.gain.setValueAtTime(0, startTime);
  gain.gain.linearRampToValueAtTime(0.3, startTime + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.001, startTime + duration);

  oscillator.start(startTime);
  oscillator.stop(startTime + duration);
}

// === 默认频率配置 ===
const DEFAULT_FREQUENCIES = {
  waiting: { primary: 880, secondary: 1174.66 },
  finished: { primary: 440 },
};

/** 从 localStorage 读取用户配置 */
function getUserFrequencies() {
  try {
    const saved = localStorage.getItem("mam-audio-frequencies");
    if (saved) {
      return { ...DEFAULT_FREQUENCIES, ...JSON.parse(saved) };
    }
  } catch {
    // ignore parse error
  }
  return DEFAULT_FREQUENCIES;
}

/** 保存用户配置到 localStorage */
export function saveUserFrequencies(config: typeof DEFAULT_FREQUENCIES) {
  localStorage.setItem("mam-audio-frequencies", JSON.stringify(config));
}

/** 获取当前频率配置 */
export function getAudioConfig() {
  return getUserFrequencies();
}

/** 等待用户输入 — 双音 chime */
export function playWaitingSound() {
  const cfg = getUserFrequencies().waiting;
  playTone(cfg.primary, 0.15, 0);
  playTone(cfg.secondary, 0.3, 0.12);
}

/** 任务完成 — 低音单音 */
export function playFinishedSound() {
  const cfg = getUserFrequencies().finished;
  playTone(cfg.primary, 0.4, 0);
}

/** 测试提示音 */
export function playTestSound() {
  playTone(660, 0.15, 0);
  playTone(880, 0.2, 0.1);
}

/** 按状态播放对应提示音 */
export function playSoundForStatus(status: string) {
  switch (status) {
    case "waiting":
      playWaitingSound();
      break;
    case "finished":
      playFinishedSound();
      break;
    default:
      break;
  }
}
