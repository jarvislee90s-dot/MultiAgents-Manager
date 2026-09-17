import { invoke } from "@tauri-apps/api/core";

// Rust 端 remote_status 的返回（serde_json::json! 裸值，键名原样，无 camelCase 重命名）
export type RemoteStatus = {
  enabled: boolean;
  bind: string;
  port: number;
  url: string;
  // 仅 bind=0.0.0.0 时非空：本机局域网地址候选（手机可直达）
  lanUrls: string[];
  // 地址表（2026-09-16 用户裁决）：设置页「访问地址」单区块逐条渲染的数据源。
  // iface = 网卡名（探测不到为空串，前端本地化兜底）；
  // primary = 推荐地址（0.0.0.0 时即默认路由那条）
  addresses: RemoteAddressEntry[];
};

export type RemoteAddressEntry = {
  url: string;
  iface: string;
  primary: boolean;
};

// remote_issue_token 的返回：一次性配对码 + 拼好 #token= 片段的可扫 URL
export type PairingToken = {
  token: string;
  url: string;
};

// 四命令定义于 src-tauri/src/remote/mod.rs（Task 4），lib.rs invoke_handler 注册
export async function remoteStatus(): Promise<RemoteStatus> {
  return await invoke<RemoteStatus>("remote_status");
}
export async function remoteToggle(enabled: boolean): Promise<void> {
  return await invoke("remote_toggle", { enabled });
}
export async function remoteIssueToken(): Promise<PairingToken> {
  return await invoke<PairingToken>("remote_issue_token");
}
// TLS 前置确认（P7 安全门）：置位 remote.public_ack，解锁 0.0.0.0 绑定
export async function remoteConfirmPublic(): Promise<void> {
  return await invoke("remote_confirm_public");
}
