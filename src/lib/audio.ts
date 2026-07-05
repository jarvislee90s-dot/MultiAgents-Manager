// Web Audio API 提示音 — 不依赖音频文件，参考 claude-control

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

/** 等待用户输入 — 双音 chime（A5 880Hz + D6 1174.66Hz） */
export function playWaitingSound() {
  playTone(880, 0.15, 0);
  playTone(1174.66, 0.3, 0.12);
}

/** 任务完成 — 低音单音（A4 440Hz） */
export function playFinishedSound() {
  playTone(440, 0.4, 0);
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
