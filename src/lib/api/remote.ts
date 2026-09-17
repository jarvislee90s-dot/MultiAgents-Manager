import { invoke } from "@tauri-apps/api/core";

// Rust 端 remote_status 的返回（serde_json::json! 裸值，键名原样，无 camelCase 重命名）。
// M5 A6：设置页切到 channels + pin 新载荷；M5 A8 回归裁决：M3 起的 legacy 键
// （bind/port/url/lanUrls/channel/tunnelUrl/tunnelError/addresses）后端已删——
// 前端唯一数据源是 enabled + maxDevices + channels + pin + host 载荷
export type RemoteStatus = {
  enabled: boolean;
  /** 设备上限（线稿「已接入设备 N / 上限」徽标；KV 可改，未设置默认 10） */
  maxDevices: number;
  // M5 A5：四通道状态 + 当前访问密码（设置页卡片与详情区的唯一数据源；
  // 形状契约见 Rust 端 channels_payload 注释——A6 卡片与 A7 移动端消费同一形状）
  channels: RemoteChannels;
  pin: string | null;
  // M3 起 host 载荷（P8a/P8b）：name = display_host_name（已存命名回落系统主机名）
  // ——A6 本机名称输入框的默认值来源
  host?: { name: string; platform?: string; version?: string; bootId?: string };
  enabledTools?: string[];
};

// M5 A5 四通道状态载荷（Rust 端 channels_payload 注释即唯一契约）：
// enabled = 三通道 KV 开关（local 无此键——随总开关常驻）；running = 运行态；
// 隧道 address 错误态或已停不宣称 → null；lan.addresses = 完整可直达 URL 列表
export type RemoteChannels = {
  local: { running: boolean; address: string };
  lan: { enabled: boolean; running: boolean; addresses: string[] };
  quick: { enabled: boolean; running: boolean; address: string | null; error: string | null };
  named: { enabled: boolean; running: boolean; address: string | null; error: string | null };
};

// 命令定义于 src-tauri/src/remote/mod.rs（M5 A1-A5 已落地）
export async function remoteStatus(): Promise<RemoteStatus> {
  return await invoke<RemoteStatus>("remote_status");
}
export async function remoteToggle(enabled: boolean): Promise<void> {
  return await invoke("remote_toggle", { enabled });
}
// 三通道独立开关（M5 A5）：channel ∈ "lan" | "quick" | "named"（本机常驻锁死无命令）。
// lan 开启未确认 TLS 反代时 Err 特征文案（PUBLIC_ACK_REQUIRED_MSG，含「对外绑定需先
// 确认已配置 TLS 反向代理」），前端据此弹确认 Dialog；幂等（重复同向调用 no-op）
export async function toggleChannel(channel: string, on: boolean): Promise<void> {
  return await invoke("remote_toggle_channel", { channel, on });
}
// 设置访问密码（M5 A4）：首次只落库；改值 = 全部设备吊销 + 断连；非法 PIN Err
export async function setPin(pin: string): Promise<void> {
  return await invoke("remote_set_pin", { pin });
}
// 重置设备（M5 A4，线稿「重置设备」按钮口径）：吊销全部设备 + SSE 断连，不改 PIN
export async function resetDevices(): Promise<void> {
  return await invoke("remote_reset_devices");
}
// 设备重命名（M5 A4）：空名拒绝；超 40 字由后端 DAO 截断
export async function renameDevice(id: string, name: string): Promise<void> {
  return await invoke("remote_rename_device", { id, name });
}
// TLS 前置确认（P7 安全门）：置位 remote.public_ack，解锁对外绑定
export async function remoteConfirmPublic(): Promise<void> {
  return await invoke("remote_confirm_public");
}

// ============================================================
// 设备花名册（已配对设备行；M5 A3 起配对仅 /pair/pin，审批/直通命令已下线）
// ============================================================

// 已配对设备（花名册行）：online 口径在后端（SSE 注册 ∨ 30s 过闸），前端只渲染。
// via = 配对时刻接入通道（local/quick/named/lan，A1 起落库）；旧载荷无此键 →
// undefined 不渲染徽标（前端不猜）
export type RemoteDevice = {
  id: string;
  name: string;
  firstPairedAt: number;
  lastSeenAt: number;
  online: boolean;
  via?: string;
};
// 设备花名册（已吊销项后端已过滤）
export async function remoteDevices(): Promise<RemoteDevice[]> {
  return await invoke<RemoteDevice[]>("remote_devices");
}
// 踢下线（线稿口径）：DB 置位 + SSE 即时断连
export async function remoteRevokeDevice(id: string): Promise<void> {
  return await invoke("remote_revoke_device", { id });
}
// 全部吊销（A6 UI 已改用 resetDevices；命令仍在后端，去留由 A8 裁决，契约保留）
export async function remoteRevokeAllDevices(): Promise<void> {
  return await invoke("remote_revoke_all_devices");
}
