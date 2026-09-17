// 设置页「远程接入」分区（M5 A6：按线稿 v5 重写，UI 唯一契约 =
// docs/superpowers/wireframes/2026-09-17-remote-settings-redesign.html）。
// 结构三段式：① 通用（总开关 / 电源保活 / 本机名称）② 通道四小卡一排 + 唯一展开
// 详情区（本机 / 局域网 / 临时隧道 / 命名隧道）③ 访问与安全（访问密码 / 重置设备 /
// 已接入设备列表）。状态唯一数据源 = remote_status 的 channels + pin 载荷（M5 A5）；
// 命令统一走 src/lib/api/remote.ts；Token / 本机名 / 保活写通用 set_setting。
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import QRCode from "qrcode";
import { Button } from "@/components/ui/button";
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
import { cn } from "@/lib/utils";
import { formatInvokeError } from "@/lib/invokeError";
import {
  remoteConfirmPublic,
  remoteDevices,
  remoteRevokeDevice,
  remoteStatus,
  remoteToggle,
  renameDevice,
  resetDevices,
  setPin,
  toggleChannel,
  type RemoteDevice,
  type RemoteStatus,
} from "@/lib/api/remote";
import { getSetting, setSetting } from "@/lib/api/settings";

// 本机名：与 Rust 端 remote::KEY_HOST_NAME 对齐；空串/空白原样写——后端
// display_host_name 过滤空白后回落系统名，前端不做非空校验（口径单点在后端）。
// A6：未设置时默认填系统名（remote_status.host.name），灰字占位提示删除
const HOST_NAME_KEY = "remote.host_name";
// 隧道 Token：与 Rust 端 remote::KEY_TUNNEL_TOKEN 对齐；A6 起保存走通用 set_setting
//（remote_set_channel 已下线），开关由命名隧道卡片开关（remote_toggle_channel）驱动
const TUNNEL_TOKEN_KEY = "remote.tunnel_token";
// 电源保活：与 Rust 端 remote::power::KEY_KEEPALIVE 对齐；默认开，
// 后端 should_acquire（None/乱串 → true）是唯一口径，前端仅同步展示
const KEEPALIVE_KEY = "remote.keepalive";
// P7 门特征文案（与 Rust PUBLIC_ACK_REQUIRED_MSG 单点常量同源的前缀特征）：前端只做
// includes 判别分流弹既有 TLS Dialog，不复制门槛判定——文案漂移由后端常量保证
const PUBLIC_ACK_FEATURE = "对外绑定需先确认已配置 TLS 反向代理";

// 四通道标识（线稿四卡顺序固定；local 随总开关常驻，无独立命令）
type ChannelKind = "local" | "lan" | "quick" | "named";
type ToggleableChannel = Exclude<ChannelKind, "local">;

// P7 特征判别：invoke 错误透传形态为 string（Rust Err(String)），兜底 Error/任意值
function isPublicAckRequired(e: unknown): boolean {
  const raw = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return raw.includes(PUBLIC_ACK_FEATURE);
}

type TFunc = ReturnType<typeof useAppTranslation>["t"];

// 设备最近活跃相对时间（分钟/小时/天三档，<1 分钟按「刚刚」）
function lastSeenLabel(lastSeenAt: number, t: TFunc): string {
  const minutes = Math.max(0, Math.floor((Date.now() - lastSeenAt) / 60_000));
  if (minutes < 1) return t("settings.remote.rosterSeenNow");
  if (minutes < 60) return t("settings.remote.rosterSeenMin", { n: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t("settings.remote.rosterSeenHour", { n: hours });
  return t("settings.remote.rosterSeenDay", { n: Math.floor(hours / 24) });
}

// 徽标（线稿 .badge：green 推荐/运行中、blue 公网、gray 仅本机/局域网、violet 隧道）
function Badge({
  tone,
  children,
}: {
  tone: "gray" | "green" | "blue" | "violet";
  children: ReactNode;
}) {
  const tones = {
    gray: "bg-secondary text-secondary-foreground",
    green: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
    blue: "bg-blue-500/15 text-blue-600 dark:text-blue-400",
    violet: "bg-violet-500/15 text-violet-600 dark:text-violet-400",
  } as const;
  return (
    <span
      className={cn(
        "inline-flex flex-none items-center rounded-full px-2 py-0.5 text-[10.5px]",
        tones[tone]
      )}
    >
      {children}
    </span>
  );
}

// 设备行 via 徽标（local/quick/named/lan → 四通道名；未知/缺失值不渲染——前端不猜）
const VIA_LABEL_KEY: Record<string, string> = {
  local: "settings.remote.chanLocal",
  lan: "settings.remote.chanLan",
  quick: "settings.remote.chanQuick",
  named: "settings.remote.chanNamed",
};
function ViaBadge({ via }: { via: string | undefined }) {
  const { t } = useAppTranslation();
  if (!via) return null;
  const key = VIA_LABEL_KEY[via];
  if (!key) return null;
  const tone = via === "quick" || via === "named" ? "violet" : "gray";
  return <Badge tone={tone}>{t(key)}</Badge>;
}

// 通道二维码（qrcode 生成放 effect 事件路径，不在渲染期调用；失败降级无图不阻塞页面）
function ChannelQr({ url, caption }: { url: string; caption: string }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    setDataUrl(null);
    QRCode.toDataURL(url, { width: 132, margin: 1 })
      .then((d) => {
        if (alive) setDataUrl(d);
      })
      .catch((e) => console.error("qrcode render failed:", e));
    return () => {
      alive = false;
    };
  }, [url]);
  return (
    <div className="flex flex-none flex-col items-center gap-1">
      {dataUrl ? (
        <img src={dataUrl} alt={caption} className="h-[132px] w-[132px] rounded-lg border" />
      ) : (
        <div className="bg-muted/40 h-[132px] w-[132px] animate-pulse rounded-lg border" />
      )}
      <p className="text-muted-foreground text-center text-[10.5px]">{caption}</p>
    </div>
  );
}

export function RemoteSection() {
  const { t } = useAppTranslation();
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  // 电源保活开关（默认开）：受控 Switch，加载回填、切换落盘
  const [keepalive, setKeepalive] = useState(true);
  // 本机名称：受控输入，加载回填、blur 落盘；未设置时默认系统名（host.name）
  const [hostName, setHostName] = useState("");
  const hostNameSavedRef = useRef(false);
  const hostNameDefaultedRef = useRef(false);
  // 访问密码：首拍 status.pin 回填一次，轮询不回读（保留编辑中值）
  const [pinInput, setPinInput] = useState("");
  const pinInitRef = useRef(false);
  // 隧道 Token：进面板回填已存值
  const [token, setToken] = useState("");
  // 教程 popover（线稿 .help：点击展开/收起，可保持展开边看边操作）
  const [helpOpen, setHelpOpen] = useState(false);
  // 唯一展开的通道详情卡（线稿 pick()：同时只显示一个，点其它卡切换）
  const [selected, setSelected] = useState<ChannelKind>("local");
  // 设备花名册（3s 轮询）+ 行内重命名态
  const [devices, setDevices] = useState<RemoteDevice[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState("");
  // TLS 对外绑定确认弹窗：lan 开关触发（toggle_channel Err 特征文案 → 确认 → 重试）
  const [tlsOpen, setTlsOpen] = useState(false);
  // 重置设备二次确认弹窗
  const [resetOpen, setResetOpen] = useState(false);
  // 开关在途互斥：连点会并发远程命令（启停竞态），与旧版 busy 语义一致
  const [busy, setBusy] = useState(false);

  // 轻量 status 刷新：只刷 status——轮询若回读 KV 会 clobber 编辑中的输入框
  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await remoteStatus());
    } catch {
      /* 刷新尽力而为，下一拍自愈 */
    }
  }, []);

  // 设备表刷新：尽力而为，失败静默（下一拍轮询自愈）；null 兜底空表
  const refreshDevices = useCallback(async () => {
    try {
      setDevices((await remoteDevices()) ?? []);
    } catch {
      /* 刷新尽力而为 */
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  // 保活回填：进面板读一次；null/非 "false" → 默认开（与后端 fail-safe 口径一致）
  useEffect(() => {
    void (async () => setKeepalive((await getSetting(KEEPALIVE_KEY)) !== "false"))();
  }, []);

  // 本机名回填：已存值优先；未设置 → 后续 effect 以系统名（status.host.name）兜底
  useEffect(() => {
    void (async () => {
      const saved = await getSetting(HOST_NAME_KEY);
      hostNameSavedRef.current = saved !== null && saved.trim() !== "";
      if (saved !== null) setHostName(saved);
    })();
  }, []);

  // 系统名兜底（A6：默认填系统用户名/主机名，去掉灰字提示）：只在「无已存值且
  // 用户尚未编辑」时填一次；ref 双闸防止轮询期间反复覆盖空输入框
  useEffect(() => {
    if (hostNameSavedRef.current || hostNameDefaultedRef.current) return;
    const sys = status?.host?.name;
    if (sys) {
      hostNameDefaultedRef.current = true;
      setHostName(sys);
    }
  }, [status]);

  // Token 回填：进面板读已存值，后续刷新不回读
  useEffect(() => {
    void (async () => setToken((await getSetting(TUNNEL_TOKEN_KEY)) ?? ""))();
  }, []);

  // PIN 回填：首个非空 status.pin 填一次（ref 闸），轮询不覆盖编辑中值
  useEffect(() => {
    if (pinInitRef.current) return;
    const p = status?.pin;
    if (typeof p === "string" && p.length > 0) {
      pinInitRef.current = true;
      setPinInput(p);
    }
  }, [status]);

  const enabled = status?.enabled ?? false;

  // 配对面板与花名册轮询：enabled 时 3s 轮询 status + devices（与移动端看板同节奏）；
  // disabled 清空设备表——不展示陈旧花名册
  useEffect(() => {
    if (!enabled) {
      setDevices([]);
      return;
    }
    void refreshDevices();
    const timer = setInterval(() => {
      void refreshStatus();
      void refreshDevices();
    }, 3000);
    return () => clearInterval(timer);
  }, [enabled, refreshDevices, refreshStatus]);

  const channels = status?.channels;
  // 卡片 live 点/开关态口径：enabled||running（本机卡 = 总开关）
  const chanOn = (key: ToggleableChannel): boolean => {
    const c = channels?.[key];
    return Boolean(c && (c.enabled || c.running));
  };
  const isOn = (key: ChannelKind): boolean => (key === "local" ? enabled : chanOn(key));

  // 二维码内容 = 通道地址 + 密码参数（线稿：链接已含密码，扫码自动填入直接进入）
  const pin = typeof status?.pin === "string" ? status.pin : "";
  const withPin = (url: string) => (pin ? `${url}#pin=${pin}` : url);
  const qrCaption = `${t("settings.remote.qrTitle")} · ${t("settings.remote.qrAutoPin")}`;

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(t("settings.remote.copied"));
    } catch (e) {
      console.error("clipboard write failed:", e);
    }
  };

  // 总开关（线稿：显性关闭 = 断开所有设备并要求重新输入访问密码，语义在行提示中）
  const changeEnabled = async (v: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      await remoteToggle(v);
      await refreshStatus();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setBusy(false);
    }
  };

  // 保活落盘：写 "true"/"false"；失败 toast 且开关回弹（受控态未变）
  const changeKeepalive = async (v: boolean) => {
    try {
      await setSetting(KEEPALIVE_KEY, v ? "true" : "false");
      setKeepalive(v);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 本机名落盘：blur 触发（不逐键写库）；空串原样写（后端回落系统名）
  const changeHostName = async (value: string) => {
    try {
      await setSetting(HOST_NAME_KEY, value);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 通道开关（线稿 tg()：开关独立、stopPropagation 不触发卡片选中）。
  // P7 门唯一口径在后端：lan 开启未确认 TLS 反代 → Err 特征文案 → 弹既有 TLS Dialog
  //（确认后重试）；其余失败 toast 原样透出。后端失败自回滚 KV，开关态以 status 为准
  const changeChannel = async (kind: ToggleableChannel, on: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      await toggleChannel(kind, on);
      await refreshStatus(); // 开关即卡关：即时刷新 live 点与地址展示
    } catch (e) {
      if (kind === "lan" && on && isPublicAckRequired(e)) {
        setTlsOpen(true);
      } else {
        toast.error(formatInvokeError(e, t));
      }
    } finally {
      setBusy(false);
    }
  };

  // TLS 确认 + 重试（仅 lan 开启路径）：先 remote_confirm_public 置位 remote.public_ack
  //（后端 P7 门据此放行），再重试 toggle_channel("lan", true)。确认失败不关弹窗
  //（可就地重试）；重试失败回落 changeChannel 既有分流（P7 已放行，异常走 toast）
  const confirmTlsAndRetryLan = async () => {
    if (busy) return;
    try {
      await remoteConfirmPublic();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
      return;
    }
    setTlsOpen(false);
    await changeChannel("lan", true);
  };

  // Token 保存（A6 落点：通用 set_setting）；开关由命名隧道卡片开关驱动，后端
  // start_channel 对空 Token 拒启并写快照错误（「命名隧道缺少 Tunnel Token」）
  const saveToken = async () => {
    try {
      await setSetting(TUNNEL_TOKEN_KEY, token);
      toast.success(t("settings.remote.tokenSaved"));
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 随机密码（线稿 randPin：1000-9999 四位数字）
  const randomPin = () => {
    setPinInput(String(Math.floor(1000 + Math.random() * 9000)));
  };

  // 密码保存：改值 = 后端全设备吊销 + 断连（「修改后所有设备需重新输入」语义）
  const savePin = async () => {
    try {
      await setPin(pinInput);
      toast.success(t("settings.remote.pinSaved"));
      await refreshStatus();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 重置设备（二次确认后）：吊销全部 + 断连，不改 PIN
  const doResetDevices = async () => {
    try {
      await resetDevices();
      setResetOpen(false);
      await refreshDevices();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  const startRename = (d: RemoteDevice) => {
    setEditingId(d.id);
    setEditName(d.name ?? "");
  };

  const saveRename = async (id: string) => {
    try {
      await renameDevice(id, editName);
      setEditingId(null);
      await refreshDevices();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 踢下线（线稿口径的单设备吊销）
  const kick = async (id: string) => {
    try {
      await remoteRevokeDevice(id);
      await refreshDevices();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    }
  };

  // 详情区数据快照（缺失键安全兜底——旧后端/异常载荷不致渲染崩溃）
  const localAddr = channels?.local?.address ?? "";
  const lanAddrs = channels?.lan?.addresses ?? [];
  const quickAddr = channels?.quick?.address ?? null;
  const quickErr = channels?.quick?.error ?? null;
  const namedAddr = channels?.named?.address ?? null;
  const namedErr = channels?.named?.error ?? null;

  // 四卡配置（线稿顺序：本机 / 局域网 / 临时隧道 / 命名隧道；desc 逐字取线稿）
  const cardDefs: Array<{ key: ChannelKind; name: string; desc: string }> = [
    {
      key: "local",
      name: t("settings.remote.chanLocal"),
      desc: t("settings.remote.chanLocalDesc"),
    },
    { key: "lan", name: t("settings.remote.chanLan"), desc: t("settings.remote.chanLanDesc") },
    {
      key: "quick",
      name: t("settings.remote.chanQuick"),
      desc: t("settings.remote.chanQuickDesc"),
    },
    {
      key: "named",
      name: t("settings.remote.chanNamed"),
      desc: t("settings.remote.chanNamedDesc"),
    },
  ];

  return (
    <div>
      <h2 className="text-lg font-semibold">{t("settings.remote.title")}</h2>
      <p className="text-muted-foreground mb-1 text-sm">{t("settings.remote.desc")}</p>

      {/* ① 通用 */}
      <div className="text-muted-foreground mt-4 text-[12.5px] font-semibold">
        {t("settings.remote.groupGeneral")}
      </div>
      <div className="flex items-center justify-between gap-4 py-3">
        <div className="flex-1">
          <label className="text-sm font-semibold">{t("settings.remote.enable")}</label>
          <p className="text-muted-foreground mt-0.5 text-xs">
            {t("settings.remote.enableCloseHint")}
          </p>
        </div>
        <Switch
          aria-label={t("settings.remote.enable")}
          checked={enabled}
          disabled={busy || !status}
          onCheckedChange={(v) => void changeEnabled(v)}
        />
      </div>
      <div className="flex items-center justify-between gap-4 py-3">
        <div className="flex-1">
          <label className="text-sm font-semibold">{t("settings.remote.keepalive")}</label>
          <p className="text-muted-foreground mt-0.5 text-xs">
            {t("settings.remote.keepaliveHint")}
          </p>
        </div>
        <Switch
          aria-label={t("settings.remote.keepalive")}
          checked={keepalive}
          disabled={!status}
          onCheckedChange={(v) => void changeKeepalive(v)}
        />
      </div>
      <div className="flex items-center justify-between gap-4 py-3">
        <label htmlFor="remote-host-name" className="flex-none text-sm font-semibold">
          {t("settings.remote.hostName")}
        </label>
        <Input
          id="remote-host-name"
          value={hostName}
          className="max-w-64"
          onChange={(e) => setHostName(e.target.value)}
          onBlur={(e) => void changeHostName(e.target.value)}
        />
      </div>

      {/* ② 通道（开关各自独立，可同时开启；点卡片在下方展开详情） */}
      <div className="text-muted-foreground mt-4 text-[12.5px] font-semibold">
        {t("settings.remote.groupChannels")}
      </div>
      <div className="mt-2 grid grid-cols-4 gap-2">
        {cardDefs.map((c) => {
          const sel = selected === c.key;
          return (
            <div
              key={c.key}
              data-card={c.key}
              role="button"
              tabIndex={0}
              aria-pressed={sel}
              onClick={() => setSelected(c.key)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") setSelected(c.key);
              }}
              className={cn(
                "cursor-pointer rounded-[10px] border-[1.5px] p-2.5 transition-colors outline-none",
                sel
                  ? "border-foreground shadow-[0_0_0_2px_rgba(15,23,42,0.07)] dark:shadow-[0_0_0_2px_rgba(255,255,255,0.09)]"
                  : "border-border hover:border-muted-foreground/60"
              )}
            >
              <div className="flex items-center justify-between gap-1.5">
                <span className="flex min-w-0 items-center gap-1 text-[13px] font-bold">
                  <span
                    data-live
                    className={cn(
                      "inline-block h-[7px] w-[7px] flex-none rounded-full",
                      isOn(c.key) ? "bg-emerald-500" : "bg-gray-300"
                    )}
                  />
                  <span className="truncate">{c.name}</span>
                </span>
                {/* 开关独立于卡片选中（线稿 stopPropagation） */}
                <span className="flex-none" onClick={(e) => e.stopPropagation()}>
                  {c.key === "local" ? (
                    // 本机卡：随总开关常驻锁死，无独立命令（后端无 local 通道）
                    <Switch
                      checked={enabled}
                      disabled
                      title={t("settings.remote.chanLocalLocked")}
                      aria-label={c.name}
                    />
                  ) : (
                    <Switch
                      checked={isOn(c.key)}
                      disabled={busy || !status}
                      aria-label={c.name}
                      onCheckedChange={(v) => void changeChannel(c.key as ToggleableChannel, v)}
                    />
                  )}
                </span>
              </div>
              <p className="text-muted-foreground mt-1 min-h-[33px] text-[11px] leading-snug">
                {c.desc}
              </p>
            </div>
          );
        })}
      </div>

      {/* 唯一展开详情区（线稿 pick()：同一时刻只显示一个） */}
      <div data-expand={selected} className="bg-muted/30 mt-2.5 rounded-xl border p-4 text-sm">
        {selected === "local" && (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Badge tone="gray">{t("settings.remote.badgeLocalOnly")}</Badge>
              {localAddr && (
                <code className="bg-muted rounded-md px-2 py-0.5 font-mono text-[12.5px] break-all">
                  {localAddr}
                </code>
              )}
              {localAddr && (
                <Button variant="outline" size="sm" onClick={() => void copy(localAddr)}>
                  {t("settings.remote.copyLink")}
                </Button>
              )}
            </div>
            <p className="text-muted-foreground mt-2 text-xs">
              {t("settings.remote.localDetailHint")}
            </p>
          </>
        )}
        {selected === "lan" && (
          <>
            <div className="flex flex-col gap-1.5">
              {lanAddrs.map((u, i) => (
                <div key={u} className="flex flex-wrap items-center gap-2">
                  {i === 0 && <Badge tone="green">{t("settings.remote.badgeRecommended")}</Badge>}
                  <code className="bg-muted rounded-md px-2 py-0.5 font-mono text-[12.5px] break-all">
                    {u}
                  </code>
                  <Button variant="outline" size="sm" onClick={() => void copy(u)}>
                    {t("settings.remote.copy")}
                  </Button>
                </div>
              ))}
            </div>
            <div className="mt-3 flex items-start gap-6">
              {lanAddrs[0] && <ChannelQr url={withPin(lanAddrs[0])} caption={qrCaption} />}
              <p className="text-muted-foreground flex-1 text-xs">
                {t("settings.remote.lanDetailHint")}
              </p>
            </div>
          </>
        )}
        {selected === "quick" && (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Badge tone="blue">{t("settings.remote.badgePublic")}</Badge>
              {quickAddr && (
                <code className="bg-muted rounded-md px-2 py-0.5 font-mono text-[12.5px] break-all">
                  {quickAddr}
                </code>
              )}
              {quickAddr && (
                <Button variant="outline" size="sm" onClick={() => void copy(quickAddr)}>
                  {t("settings.remote.copy")}
                </Button>
              )}
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              {channels?.quick?.running && (
                <>
                  <Badge tone="green">{t("settings.remote.badgeRunning")}</Badge>
                  <span className="text-muted-foreground text-xs">
                    {t("settings.remote.autoReconnect")}
                  </span>
                </>
              )}
              {quickErr && <span className="text-xs text-amber-500">{quickErr}</span>}
            </div>
            {/* 换址警告块（线稿 .warn，逐字） */}
            <p className="mt-2 rounded-lg bg-amber-50 px-2.5 py-1.5 text-xs text-amber-600 dark:bg-amber-500/10 dark:text-amber-400">
              {t("settings.remote.quickWarn")}
            </p>
            <div className="mt-3 flex items-start gap-6">
              {quickAddr && <ChannelQr url={withPin(quickAddr)} caption={qrCaption} />}
              <p className="text-muted-foreground flex-1 text-xs">
                {t("settings.remote.quickDetailHint")}
              </p>
            </div>
          </>
        )}
        {selected === "named" && (
          <>
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 flex-1">
                <div className="text-[12.5px] font-semibold">
                  {t("settings.remote.tunnelToken")}{" "}
                  {/* 教程 popover（线稿 .help：点击展开/收起，可保持展开边看边操作） */}
                  <span className="relative inline-block align-baseline">
                    <span
                      role="button"
                      tabIndex={0}
                      data-help={helpOpen ? "open" : "closed"}
                      className="cursor-pointer border-b border-dotted border-blue-500 text-xs text-blue-500"
                      onClick={() => setHelpOpen((v) => !v)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") setHelpOpen((v) => !v);
                      }}
                    >
                      {t("settings.remote.tutorialToggle")}
                    </span>
                    {helpOpen && (
                      <div className="bg-background absolute top-6 left-[-40px] z-30 w-[420px] rounded-xl border p-3.5 text-xs shadow-lg">
                        <p className="font-semibold">{t("settings.remote.tutorialPre")}</p>
                        <ol className="mt-1.5 list-decimal space-y-1.5 pl-4">
                          <li>{t("settings.remote.tutorialStep1")}</li>
                          <li>{t("settings.remote.tutorialStep2")}</li>
                          <li>{t("settings.remote.tutorialStep3")}</li>
                          <li>{t("settings.remote.tutorialStep4")}</li>
                          <li>{t("settings.remote.tutorialStep5")}</li>
                          <li>{t("settings.remote.tutorialStep6")}</li>
                        </ol>
                        <p className="text-muted-foreground mt-2">
                          {t("settings.remote.tutorialPost")}
                        </p>
                      </div>
                    )}
                  </span>
                </div>
                <p className="text-muted-foreground mt-0.5 text-xs">
                  {t("settings.remote.tokenHint")}
                </p>
              </div>
              <div className="flex max-w-[300px] flex-none gap-2">
                <Input
                  id="remote-tunnel-token"
                  type="password"
                  value={token}
                  aria-label={t("settings.remote.tunnelToken")}
                  className="flex-1"
                  onChange={(e) => setToken(e.target.value)}
                />
                <Button size="sm" onClick={() => void saveToken()}>
                  {t("settings.remote.save")}
                </Button>
              </div>
            </div>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Badge tone="blue">{t("settings.remote.badgePublic")}</Badge>
              {namedAddr && (
                <code className="bg-muted rounded-md px-2 py-0.5 font-mono text-[12.5px] break-all">
                  {namedAddr}
                </code>
              )}
              {namedAddr && (
                <Button variant="outline" size="sm" onClick={() => void copy(namedAddr)}>
                  {t("settings.remote.copy")}
                </Button>
              )}
              {namedErr && <span className="text-xs text-amber-500">{namedErr}</span>}
            </div>
            {namedAddr && (
              <div className="mt-3">
                <ChannelQr url={withPin(namedAddr)} caption={qrCaption} />
              </div>
            )}
          </>
        )}
      </div>

      {/* ③ 访问与安全 */}
      <div className="text-muted-foreground mt-4 text-[12.5px] font-semibold">
        {t("settings.remote.groupSecurity")}
      </div>
      <div className="flex items-center justify-between gap-4 py-3">
        <div className="flex-1">
          <label htmlFor="remote-pin" className="text-sm font-semibold">
            {t("settings.remote.pinTitle")}
          </label>
          <p className="text-muted-foreground mt-0.5 text-xs">{t("settings.remote.pinHint")}</p>
        </div>
        <div className="flex flex-none items-center gap-2.5">
          <Input
            id="remote-pin"
            inputMode="numeric"
            maxLength={4}
            placeholder="0000"
            value={pinInput}
            onChange={(e) => setPinInput(e.target.value.replace(/\D/g, "").slice(0, 4))}
            className="w-28 text-center font-mono text-lg font-bold tracking-[0.3em]"
          />
          <Button variant="outline" size="sm" onClick={randomPin}>
            {t("settings.remote.pinRandom")}
          </Button>
          <Button size="sm" onClick={() => void savePin()}>
            {t("settings.remote.save")}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="text-rose-600 hover:text-rose-600"
            onClick={() => setResetOpen(true)}
          >
            {t("settings.remote.resetDevices")}
          </Button>
        </div>
      </div>
      <div className="py-3">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold">{t("settings.remote.devicesTitle")}</span>
          {/* 上限徽标（决策 #17：上限默认 10） */}
          <Badge tone="gray">{`${devices.length} / 10`}</Badge>
        </div>
        <p className="text-muted-foreground mt-0.5 text-xs">{t("settings.remote.devicesHint")}</p>
        {devices.length === 0 ? (
          <p className="text-muted-foreground mt-2 text-xs">{t("settings.remote.rosterEmpty")}</p>
        ) : (
          <ul className="mt-1">
            {devices.map((d) => (
              <li
                key={d.id}
                className="flex flex-wrap items-center gap-2.5 py-2 text-[13px] [&:not(:last-child)]:border-b [&:not(:last-child)]:border-dashed"
              >
                <span
                  className={cn(
                    "h-2 w-2 flex-none rounded-full",
                    d.online ? "bg-green-500" : "bg-gray-300"
                  )}
                />
                {editingId === d.id ? (
                  <>
                    <Input
                      aria-label={t("settings.remote.rename")}
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      className="h-7 w-44 px-2 text-xs"
                    />
                    <Button size="sm" onClick={() => void saveRename(d.id)}>
                      {t("settings.remote.save")}
                    </Button>
                  </>
                ) : (
                  <>
                    <span className="max-w-48 truncate">{d.name || d.id.slice(0, 8)}</span>
                    <ViaBadge via={d.via} />
                    <span className="text-muted-foreground text-xs">
                      {d.online && (
                        <span className="text-emerald-600">
                          {t("settings.remote.rosterOnline")}
                        </span>
                      )}
                      {d.online ? " · " : ""}
                      {lastSeenLabel(d.lastSeenAt, t)}
                    </span>
                    <Button variant="outline" size="sm" onClick={() => startRename(d)}>
                      {t("settings.remote.rename")}
                    </Button>
                  </>
                )}
                <Button
                  variant="outline"
                  size="sm"
                  className="ml-auto text-rose-600 hover:text-rose-600"
                  onClick={() => void kick(d.id)}
                >
                  {t("settings.remote.kick")}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>
      {/* 底部安全警示（线稿：地址+密码即钥匙勿外传；连错 5 次锁 10 分钟） */}
      <p className="mt-1 rounded-lg bg-amber-50 px-2.5 py-1.5 text-xs text-amber-600 dark:bg-amber-500/10 dark:text-amber-400">
        {t("settings.remote.notice")}
      </p>

      {/* 重置设备二次确认弹窗（对齐仓库既有 Dialog 组件） */}
      <Dialog open={resetOpen} onOpenChange={setResetOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("settings.remote.resetTitle")}</DialogTitle>
            <DialogDescription>{t("settings.remote.resetDesc")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setResetOpen(false)}>
              {t("settings.remote.cancel")}
            </Button>
            <Button variant="destructive" onClick={() => void doResetDevices()}>
              {t("settings.remote.resetConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* TLS 对外绑定确认弹窗（既有 P7 门槛 UI）：lan 开关触发，确认 = 置位
          remote.public_ack（长期生效）并重试开启 lan；取消仅关弹窗、不调后端 */}
      <Dialog open={tlsOpen} onOpenChange={setTlsOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("settings.remote.tlsDialogTitle")}</DialogTitle>
            <DialogDescription>{t("settings.remote.tlsDialogDesc")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setTlsOpen(false)}>
              {t("settings.remote.cancel")}
            </Button>
            <Button onClick={() => void confirmTlsAndRetryLan()} disabled={busy}>
              {t("settings.remote.tlsDialogConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
