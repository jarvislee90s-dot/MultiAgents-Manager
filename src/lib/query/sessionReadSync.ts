// 跨窗口已读同步（T1）：看板与宠物是独立 WebView、各持状态——看板点掉未读卡
// （X 关闭或跳转成功）后，后端 `mark_session_read` / `focus_session` 广播
// "session-read"，宠物窗口凭此事件对本地状态机做已读置位，卡片行为与看板一致
// （spec W4 已读信号：跳转/关闭已读广播至宠物等辅助窗口）
import { listen } from "@tauri-apps/api/event";

/** 后端 session-read 事件 payload（serde camelCase） */
export interface SessionReadPayload {
  agentType: string;
  sessionId: string;
}

/** 注册 session-read 监听：事件到达 → 回调 (agentType, sessionId)。
 * 返回 unlisten 函数（组件卸载时清理）；非 Tauri 环境（浏览器/mock）静默空操作 */
export async function setupSessionReadListener(
  onRead: (agentType: string, sessionId: string) => void
): Promise<() => void> {
  try {
    return await listen<SessionReadPayload>("session-read", (event) => {
      const { agentType, sessionId } = event.payload ?? ({} as Partial<SessionReadPayload>);
      if (typeof agentType === "string" && typeof sessionId === "string") {
        onRead(agentType, sessionId);
      }
    });
  } catch {
    return () => {};
  }
}
