// 设置页「远程接入」分区（M2 Task 7）：开关 / 绑定二选 / 地址与局域网候选 /
// 配对二维码 / TLS 反代确认 / 停止确认弹窗 / 底部安全警示。
// 四命令统一走 src/lib/api/remote.ts；绑定写值走通用 set_setting（commands/settings.rs）。
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
  remoteStatus,
  remoteToggle,
  type PairingToken,
  type RemoteStatus,
} from "@/lib/api/remote";
import { getSetting, setSetting } from "@/lib/api/settings";

// 与 Rust 端 remote::KEY_BIND / KEY_PUBLIC_ACK 对齐的设置键（漂移即读写错位）
const BIND_KEY = "remote.bind";
const PUBLIC_ACK_KEY = "remote.public_ack";
// 本机名（P8b 收尾）：与 Rust 端 remote::KEY_HOST_NAME 对齐；空串/空白原样写——
// 后端 display_host_name 过滤空白后回落系统名，前端不做非空校验（口径单点在后端）
const HOST_NAME_KEY = "remote.host_name";
// 绑定只允许这两个字面量（Task 4 评审：后端 TLS 门只匹配 "0.0.0.0"；若允许自由输入
// 具体局域网 IP，会绕过「对外必须先确认 TLS 反代」的 ack 门）
const BIND_LOCAL = "127.0.0.1";
const BIND_LAN = "0.0.0.0";

export function RemoteSection() {
  const { t } = useAppTranslation();
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  // TLS 反代确认位（remote.public_ack）；remote_status 不含 ack，单独读 KV
  const [acked, setAcked] = useState(false);
  // 本机名（P8b 收尾）：受控输入，加载回填、blur 落盘（不逐键写库）
  const [hostName, setHostName] = useState("");
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

  const enabled = status?.enabled ?? false;
  const bind = status?.bind ?? BIND_LOCAL;

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

  const lanUrls = status?.lanUrls ?? [];

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

        {/* TLS 反代确认：仅对外绑定（0.0.0.0）时出现；勾选即调 remote_confirm_public 置位 */}
        {bind === BIND_LAN && (
          <>
            <div className="flex items-center justify-between py-2.5">
              <label className="text-sm font-medium">{t("settings.remote.tlsAck")}</label>
              <Checkbox
                checked={acked}
                onCheckedChange={(v) => {
                  if (v === true) void confirmPublic();
                }}
              />
            </div>
            <div className="border-t" />
          </>
        )}

        {/* 运行中才展示地址与二维码（停止状态无服务可连） */}
        {enabled && status && (
          <>
            <div className="flex items-center justify-between py-2.5">
              <label className="text-sm font-medium">{t("settings.remote.address")}</label>
              <div className="flex items-center gap-2">
                <code className="text-xs">{status.url}</code>
                <Button variant="outline" size="sm" onClick={() => void copy(status.url)}>
                  <Copy className="mr-1 h-3 w-3" />
                  {t("settings.remote.copy")}
                </Button>
              </div>
            </div>
            {/* lanUrls 仅 0.0.0.0 绑定时非空（后端 lan_hosts_for 门控） */}
            {lanUrls.length > 0 && (
              <>
                <div className="border-t" />
                <div className="flex items-center justify-between py-2.5">
                  <label className="text-sm font-medium">{t("settings.remote.lanAddress")}</label>
                  <div className="flex flex-col items-end gap-1">
                    {lanUrls.map((u) => (
                      <div key={u} className="flex items-center gap-2">
                        <code className="text-xs">{u}</code>
                        <Button variant="outline" size="sm" onClick={() => void copy(u)}>
                          <Copy className="mr-1 h-3 w-3" />
                          {t("settings.remote.copy")}
                        </Button>
                      </div>
                    ))}
                  </div>
                </div>
              </>
            )}
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

        {/* 底部安全警示（简报固定文案两条） */}
        <div className="border-t" />
        <div className="space-y-1 py-2.5">
          <p className="text-xs text-amber-500">{t("settings.remote.notice1")}</p>
          <p className="text-muted-foreground text-xs">{t("settings.remote.notice2")}</p>
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
