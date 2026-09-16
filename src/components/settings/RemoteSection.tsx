// 设置页「远程接入」分区（M2 Task 7）：开关 / 绑定二选 / 本机名 / 外部通道三选（M4 T1a）/
// 地址与局域网候选 / 配对二维码 / TLS 反代确认 / 停止确认弹窗 / 底部安全警示。
// 四命令统一走 src/lib/api/remote.ts；绑定写值走通用 set_setting（commands/settings.rs）；
// 通道写值走 remote_set_channel（remote/mod.rs，M4 T1a 热切换）。
import { useCallback, useEffect, useState } from "react";
import QRCode from "qrcode";
import { Copy, QrCode, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useAppTranslation } from "@/hooks/use-app-translation";
import { toast } from "sonner";
import { formatInvokeError } from "@/lib/invokeError";
import {
  remoteConfirmPublic,
  remoteIssueToken,
  remoteSetChannel,
  remoteStatus,
  remoteToggle,
  type PairingToken,
  type RemoteAddressEntry,
  type RemoteStatus,
} from "@/lib/api/remote";
import { getSetting, setSetting } from "@/lib/api/settings";

// 与 Rust 端 remote::KEY_BIND / KEY_PUBLIC_ACK 对齐的设置键（漂移即读写错位）
const BIND_KEY = "remote.bind";
const PUBLIC_ACK_KEY = "remote.public_ack";
// 本机名（P8b 收尾）：与 Rust 端 remote::KEY_HOST_NAME 对齐；空串/空白原样写——
// 后端 display_host_name 过滤空白后回落系统名，前端不做非空校验（口径单点在后端）
const HOST_NAME_KEY = "remote.host_name";
// 隧道 Token（M4 T1a）：与 Rust 端 remote::KEY_TUNNEL_TOKEN 对齐；named 通道前置条件
const TUNNEL_TOKEN_KEY = "remote.tunnel_token";
// 绑定只允许这两个字面量（Task 4 评审：后端 TLS 门只匹配 "0.0.0.0"；若允许自由输入
// 具体局域网 IP，会绕过「对外必须先确认 TLS 反代」的 ack 门）
const BIND_LOCAL = "127.0.0.1";
const BIND_LAN = "0.0.0.0";
// 外部通道三值（M4 T1a）：与 Rust 端 tunnel::parse_channel 值域对齐
const CHANNEL_OFF = "off";
const CHANNEL_QUICK = "quick";
const CHANNEL_NAMED = "named";

export function RemoteSection() {
  const { t } = useAppTranslation();
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  // TLS 反代确认位（remote.public_ack）；remote_status 不含 ack，单独读 KV
  const [acked, setAcked] = useState(false);
  // 本机名（P8b 收尾）：受控输入，加载回填、blur 落盘（不逐键写库）
  const [hostName, setHostName] = useState("");
  // 隧道 Token（M4 T1a）：named 通道的受控输入；进面板回填已存值
  const [token, setToken] = useState("");
  const [pairing, setPairing] = useState<PairingToken | null>(null);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [stopOpen, setStopOpen] = useState(false);
  // 开关在途互斥：连点会并发 remote_toggle（start/stop 竞态），与设置页 toolSaving 同型
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await remoteStatus());
      setAcked((await getSetting(PUBLIC_ACK_KEY)) === "true");
      // 本机名回填（P8b 收尾）：null（从未设置）→ 空输入框，占位符提示格式
      setHostName((await getSetting(HOST_NAME_KEY)) ?? "");
    } catch (e) {
      console.error("remote_status failed:", e);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // 回填（M4 T1a）：进面板时读已存 Tunnel Token（named 输入框免空）；后续 refresh
  // 不回读，保留用户正在编辑的值
  useEffect(() => {
    void (async () => setToken((await getSetting(TUNNEL_TOKEN_KEY)) ?? ""))();
  }, []);

  const enabled = status?.enabled ?? false;
  const bind = status?.bind ?? BIND_LOCAL;
  // 通道（M4 T1a）：status 未加载或旧后端无 channel 键 → 按 off 展示（undefined 兜底）
  const channel = status?.channel ?? CHANNEL_OFF;

  // 开启：后端有 P7 安全门（0.0.0.0 未确认 TLS 反代 → Err），失败 toast 原样透出，
  // 前端不复制门槛判定（避免与后端双源漂移）
  const enable = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await remoteToggle(true);
      await refresh(); // 开=刷新地址展示（简报 Step 2）
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const disable = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await remoteToggle(false);
      await refresh();
      // 停止 = 全吊销（不变量 5）：已展示的配对码/二维码立即作废，不给过期凭据假象
      setPairing(null);
      setQrDataUrl(null);
      setStopOpen(false);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const changeBind = async (value: string) => {
    if (value === bind) return;
    try {
      await setSetting(BIND_KEY, value);
      // 不自动重启（控制者裁决 3）：绑定变更不热生效，运行中由 bindRestartHint 提示
      await refresh();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 通道热切换（M4 T1a）：named 时随调携 Token；named 无 Token 由后端拒绝——
  // Err 中文文案原样 toast，前端不复制门槛判定（避免与后端双源漂移）
  const changeChannel = async (mode: string) => {
    if (mode === channel) return;
    try {
      await remoteSetChannel(mode, mode === CHANNEL_NAMED ? token : undefined);
      await refresh();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // Token 保存（M4 T1a）：走同一 remote_set_channel（后端先写 Token 后校验落库），
  // 成功即以 named 通道生效并 toast 确认
  const saveToken = async () => {
    try {
      await remoteSetChannel(CHANNEL_NAMED, token);
      await refresh();
      toast.success(t("settings.remote.channelSaved"));
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 本机名落盘（P8b 收尾）：blur 触发（不逐键写库）；原样写不做非空校验——
  // 空串/空白由后端 display_host_name 过滤后回落系统名，口径单点保留在后端
  const changeHostName = async (value: string) => {
    try {
      await setSetting(HOST_NAME_KEY, value);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  const confirmPublic = async () => {
    try {
      await remoteConfirmPublic();
      setAcked(true);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // qrcode 只在事件处理器中生成 dataURL（不在渲染期调用）；单活跃 token：
  // 重新生成即作废旧码（不变量 1），前端直接覆盖展示
  const issueQr = async () => {
    try {
      const tok = await remoteIssueToken();
      const dataUrl = await QRCode.toDataURL(tok.url, { width: 192, margin: 1 });
      setPairing(tok);
      setQrDataUrl(dataUrl);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(t("settings.remote.copied"));
    } catch (e) {
      console.error("clipboard write failed:", e);
    }
  };

  // 地址表（2026-09-16 用户裁决）：单区块逐条渲染。addresses 缺失时（后端旧版
  // 或载荷异常）回落单条 status.url，保证展示不空
  const addresses: RemoteAddressEntry[] =
    status?.addresses && status.addresses.length > 0
      ? status.addresses
      : status
        ? [{ url: status.url, iface: "", primary: true }]
        : [];

  return (
    <div className="space-y-4">
      <div>
        <h2 className="mb-1 text-lg font-semibold">{t("settings.remote.title")}</h2>
        <p className="text-muted-foreground text-sm">{t("settings.remote.desc")}</p>
      </div>

      <div className="space-y-0">
        {/* 开关：关=先弹停止确认（断开全部设备，简报 Step 2） */}
        <div className="flex items-center justify-between py-2.5">
          <div className="flex-1">
            <label className="text-sm font-medium">{t("settings.remote.enable")}</label>
          </div>
          <Switch
            checked={enabled}
            disabled={busy || !status}
            onCheckedChange={(v) => (v ? void enable() : setStopOpen(true))}
          />
        </div>
        <div className="border-t" />

        {/* 绑定二选：只能 127.0.0.1 / 0.0.0.0 两个字面量（Task 8 验收场景前提） */}
        <div className="flex items-center justify-between py-2.5">
          <div className="flex-1">
            <label className="text-sm font-medium">{t("settings.remote.bind")}</label>
            {enabled && (
              <p className="text-muted-foreground mt-0.5 text-xs">
                {t("settings.remote.bindRestartHint")}
              </p>
            )}
          </div>
          <div className="flex gap-2">
            <Button
              variant={bind === BIND_LOCAL ? "default" : "outline"}
              size="sm"
              onClick={() => void changeBind(BIND_LOCAL)}
            >
              {t("settings.remote.bindLocal")} (127.0.0.1)
            </Button>
            <Button
              variant={bind === BIND_LAN ? "default" : "outline"}
              size="sm"
              onClick={() => void changeBind(BIND_LAN)}
            >
              {t("settings.remote.bindLan")} (0.0.0.0)
            </Button>
          </div>
        </div>
        <div className="border-t" />

        {/* 本机名（P8b 收尾）：移动看板品牌行右侧展示的双机辨识名。受控输入 blur 落盘；
            空串原样写（后端过滤空白回落系统名），占位符给出示例格式 */}
        <div className="flex items-center justify-between gap-4 py-2.5">
          <div className="flex-1">
            <label htmlFor="remote-host-name" className="text-sm font-medium">
              {t("settings.remote.hostName")}
            </label>
          </div>
          <Input
            id="remote-host-name"
            value={hostName}
            placeholder={t("settings.remote.hostNamePlaceholder")}
            className="w-56"
            onChange={(e) => setHostName(e.target.value)}
            onBlur={(e) => void changeHostName(e.target.value)}
          />
        </div>
        <div className="border-t" />

        {/* 外部通道（M4 T1a，2026-09-16 裁决：独立区块与绑定并存）：off/quick/named 三选。
            quick/named 切换即后端热生效（restart_if_running）；named 需先填 Tunnel Token
            （门槛在后端，Err 文案原样 toast）。下方随 status 展示隧道状态：错误黄字、
            成功给当前隧道地址 */}
        <div className="flex items-center justify-between gap-4 py-2.5">
          <div className="flex-1">
            <label className="text-sm font-medium">{t("settings.remote.channel")}</label>
            <p className="text-muted-foreground mt-0.5 text-xs">
              {t("settings.remote.channelHint")}
            </p>
          </div>
          <div className="flex gap-2">
            <Button
              variant={channel === CHANNEL_OFF ? "default" : "outline"}
              size="sm"
              onClick={() => void changeChannel(CHANNEL_OFF)}
            >
              {t("settings.remote.channelOff")}
            </Button>
            <Button
              variant={channel === CHANNEL_QUICK ? "default" : "outline"}
              size="sm"
              onClick={() => void changeChannel(CHANNEL_QUICK)}
            >
              {t("settings.remote.channelQuick")}
            </Button>
            <Button
              variant={channel === CHANNEL_NAMED ? "default" : "outline"}
              size="sm"
              onClick={() => void changeChannel(CHANNEL_NAMED)}
            >
              {t("settings.remote.channelNamed")}
            </Button>
          </div>
        </div>
        {channel !== CHANNEL_OFF && <div className="border-t" />}
        {channel === CHANNEL_NAMED && (
          <div className="flex items-center justify-between gap-4 py-2.5">
            <div className="flex-1">
              <label htmlFor="remote-tunnel-token" className="text-sm font-medium">
                {t("settings.remote.tunnelToken")}
              </label>
              <p className="text-muted-foreground mt-0.5 text-xs">
                {t("settings.remote.tunnelTokenHint")}
              </p>
            </div>
            <div className="flex w-72 gap-2">
              <Input
                id="remote-tunnel-token"
                value={token}
                type="password"
                placeholder="eyJh...（Cloudflare Tunnel Token）"
                className="flex-1"
                onChange={(e) => setToken(e.target.value)}
              />
              <Button size="sm" variant="outline" onClick={() => void saveToken()}>
                {t("settings.remote.channelSave")}
              </Button>
            </div>
          </div>
        )}
        {status?.tunnelError && <p className="pb-2 text-xs text-amber-500">{status.tunnelError}</p>}
        {status?.tunnelUrl && (
          <p className="text-muted-foreground pb-2 text-xs break-all">
            {t("settings.remote.tunnelCurrent")}: {status.tunnelUrl}
          </p>
        )}
        <div className="border-t" />

        {/* TLS 反代确认：仅对外绑定（0.0.0.0）时出现；勾选即调 remote_confirm_public 置位。
            M4 T0b：确认只进不退——取消方向回弹（受控于 acked 态天然回弹）并 toast 明示
            不可在线撤回；复选框下方常驻说明撤销路径（改绑本机模式） */}
        {bind === BIND_LAN && (
          <>
            <div className="flex items-center justify-between py-2.5">
              <label className="text-sm font-medium">{t("settings.remote.tlsAck")}</label>
              <Checkbox
                checked={acked}
                onCheckedChange={(v) => {
                  if (v === true) void confirmPublic();
                  else
                    // 分隔符不硬编码：句号收进各 locale 文案（评审 Important：英文 locale 下
                    // 全角「。」混排是用户可见 i18n 缺陷），组件仅以空格连接两句
                    toast.info(
                      `${t("settings.remote.tlsAckNoRevoke")} ${t("settings.remote.tlsAckRevokeByRebind")}`
                    );
                }}
              />
            </div>
            <p className="text-muted-foreground pb-2 text-xs">{t("settings.remote.tlsAckHint")}</p>
            <div className="border-t" />
          </>
        )}

        {/* 运行中才展示地址与二维码（停止状态无服务可连） */}
        {enabled && status && (
          <>
            {/* 地址区块（2026-09-16 用户裁决）：单区块逐条展示全部可用地址——
                多网卡机器有多个网段（如实测 WLAN 与以太网），旧版「访问地址 +
                局域网地址」两块并列会被读成重复。每条标注网卡名，主/推荐那条
                带标记；复制按钮逐条给（手机对准所在网络挑一条即可） */}
            <div className="flex items-start justify-between gap-4 py-2.5">
              <label className="mt-0.5 shrink-0 text-sm font-medium">
                {t("settings.remote.address")}
              </label>
              <div className="flex flex-col items-end gap-1.5">
                {addresses.map((a) => (
                  <div key={a.url} className="flex items-center gap-2">
                    {/* M4 T1a：隧道条目 iface 恒空串——若走「本机」兜底会与「外部通道」
                        徽标并排自相矛盾，故 kind=tunnel 隐藏网卡名兜底只出徽标；
                        旧后端无 kind（undefined ≠ tunnel）→ 照旧渲染 */}
                    {a.kind !== "tunnel" && (
                      <span className="text-muted-foreground text-xs">
                        {a.iface || t("settings.remote.addressLocal")}
                      </span>
                    )}
                    {a.kind === "tunnel" && (
                      <span className="rounded-full bg-blue-500/15 px-1.5 py-0.5 text-[10px] text-blue-600 dark:text-blue-400">
                        {t("settings.remote.channelBadge")}
                      </span>
                    )}
                    {a.primary && (
                      <span className="rounded-full bg-emerald-500/15 px-1.5 py-0.5 text-[10px] text-emerald-600 dark:text-emerald-400">
                        {t("settings.remote.addressRecommended")}
                      </span>
                    )}
                    <code className="text-xs">{a.url}</code>
                    <Button variant="outline" size="sm" onClick={() => void copy(a.url)}>
                      <Copy className="mr-1 h-3 w-3" />
                      {t("settings.remote.copy")}
                    </Button>
                  </div>
                ))}
              </div>
            </div>
            <div className="border-t" />
            <div className="flex items-center justify-between py-2.5">
              <label className="text-sm font-medium">{t("settings.remote.qrGenerate")}</label>
              <Button
                size="sm"
                variant={pairing ? "outline" : "default"}
                onClick={() => void issueQr()}
              >
                {pairing ? (
                  <RefreshCw className="mr-1 h-3 w-3" />
                ) : (
                  <QrCode className="mr-1 h-3.5 w-3.5" />
                )}
                {pairing ? t("settings.remote.qrRegenerate") : t("settings.remote.qrGenerate")}
              </Button>
            </div>
            {pairing && qrDataUrl && (
              <div className="flex flex-col items-center gap-2 pb-2.5">
                <img src={qrDataUrl} alt={pairing.url} className="h-48 w-48 rounded-md border" />
                <code className="text-muted-foreground text-center text-xs break-all">
                  {pairing.url}
                </code>
                <p className="text-muted-foreground text-xs">{t("settings.remote.qrHint")}</p>
              </div>
            )}
          </>
        )}

        {/* 底部安全警示（简报固定文案两条）+ Tailscale 指引（M4 T1a） */}
        <div className="border-t" />
        <div className="space-y-1 py-2.5">
          <p className="text-xs text-amber-500">{t("settings.remote.notice1")}</p>
          <p className="text-muted-foreground text-xs">{t("settings.remote.notice2")}</p>
          <p className="text-muted-foreground text-xs">{t("settings.remote.tailscaleHint")}</p>
        </div>
      </div>

      {/* 停止确认弹窗：「停止将断开全部设备」 */}
      <Dialog open={stopOpen} onOpenChange={setStopOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("settings.remote.stopTitle")}</DialogTitle>
            <DialogDescription>{t("settings.remote.stopDesc")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setStopOpen(false)}>
              {t("settings.remote.cancel")}
            </Button>
            <Button variant="destructive" onClick={() => void disable()} disabled={busy}>
              {t("settings.remote.stopConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
