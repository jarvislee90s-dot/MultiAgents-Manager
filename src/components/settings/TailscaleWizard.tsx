// §C2 Tailscale 首次配置引导一条龙（Task 6）+ §C3 强制可达性校验呈现（Task 7）：
// 按后端 wizard_steps 渲染纵向步骤列表，每步三态（待做 / 进行中 / 已完成）；需要人
// 的步骤突出显示用户要做什么（actAdmin 等动作文案键由后端下发）；卡住时明确显示该步
// blocked_reason——**不得只转圈**。Task 7：verify 行承接可达性校验结果——Failed 时
// 显示「固定地址尚未生效」+ reason + 重试按钮（调 remote_ts_run_step("verify")，后端
// 自动含自愈）；头部看板地址只在 reach=Verified 时显示（未验证不得当可用地址展示）；
// 动作失败（Err reject）落到对应步骤行的可见错误行——**不弹全局 toast、也不静默**。
// Windows 行如实标注「尚未实机校验」弱提示（数据源 windowsVerified）——不把未验证的
// 流程伪装成已验证。数据源 = remote_ts_probe / remote_ts_run_step（形状契约见
// Rust 端 tailscale::wizard_status / run_step 注释）。
// 导出组件：TailscaleWizard（步骤列表本体，RemoteSection 的 tailscale 卡详情区
// 直接挂载——Task 9 卡片重排时已收敛，旧的独立入口行已撤）。tsWizard.* 两级键的
// 字面量必须只出现在本文件：RemoteSection 的 i18n 守卫测试按字面量扫描单层键，
// 两级键由本文件承担（本目录 tailscaleWizard.test.tsx 以两级感知扫描把关双语言键齐备）。
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { SETTINGS_NOTE, SETTINGS_REMOTE_BADGE } from "@/components/settings/typography";
import { useAppTranslation } from "@/hooks/use-app-translation";
import { cn } from "@/lib/utils";
import {
  remoteTsProbe,
  remoteTsRunStep,
  type TsStepState,
  type TsWizardProbe,
  type TsWizardStep,
} from "@/lib/api/remote";

// 步骤标题 i18n 键（显式映射而非模板串拼接——键必须字面存在，两级感知扫描才把得了关）
const STEP_KEY: Record<string, string> = {
  detect: "settings.remote.tsWizard.step_detect",
  download: "settings.remote.tsWizard.step_download",
  install: "settings.remote.tsWizard.step_install",
  sys_ext: "settings.remote.tsWizard.step_sys_ext",
  login: "settings.remote.tsWizard.step_login",
  shields_up: "settings.remote.tsWizard.step_shields_up",
  funnel: "settings.remote.tsWizard.step_funnel",
  verify: "settings.remote.tsWizard.step_verify",
  autostart: "settings.remote.tsWizard.step_autostart",
};

// 可自动触发的步（id 与 Rust wizard_steps 一致）。login 不在列：MAM 不代登录，
// 动作是「去登录」链接；detect/sys_ext 是只读探针；verify 走下方专属「重试」按钮
//（§C3，未验/失败都要能重触发校验）；autostart 是 MAM 自身行为（恒已完成）
const RUNNABLE = new Set(["download", "install", "shields_up", "funnel"]);

// **M3（2026-10-07 评审 Minor）**：线稿状态二的两行琥珀提示（wireframe 354-355：
// 「恢复中」「发布中」）**带 badge**，与状态五同形（`b-amber` = 正在进行、无需操作的
// 中间态，既不是故障 rose 也不是完成 green）；实现侧此前是纯文本 `<p>`，线稿与实现
// 各说各话。本组件内联同形徽标（**不 import RemoteSection 的 Badge**：RemoteSection
// 已 import 本组件，反向 import 会成环；字号走 `SETTINGS_REMOTE_BADGE` 这一份线稿档，
// 与那份 amber 档逐字一致）。
function AmberBadge({ children }: { children: React.ReactNode }) {
  return (
    <span
      className={cn(
        "inline-flex flex-none items-center rounded-full bg-amber-500/15 px-2 py-0.5 text-amber-600 dark:text-amber-400",
        SETTINGS_REMOTE_BADGE
      )}
    >
      {children}
    </span>
  );
}

export function TailscaleWizard() {
  const { t } = useAppTranslation();
  const [probe, setProbe] = useState<TsWizardProbe | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // 在途步互斥：连点会并发远程命令（下载/安装竞态）
  const [busyStep, setBusyStep] = useState<string | null>(null);
  // funnel 步回执带出的批准链接（首次开通时上游要求一次浏览器批准；MAM 只递不代点）
  const [approvalUrl, setApprovalUrl] = useState<string | null>(null);
  // 动作失败的落点（Task 6 评审移交指针）：按步骤行内展示，不再 catch 吞错零反馈
  const [stepError, setStepError] = useState<{ step: string; message: string } | null>(null);

  const load = useCallback(async () => {
    try {
      setProbe(await remoteTsProbe());
      setLoadError(null);
    } catch (e) {
      setLoadError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (step: string) => {
    if (busyStep) return;
    setBusyStep(step);
    setStepError(null);
    try {
      const r = await remoteTsRunStep(step);
      if (r.approvalUrl) setApprovalUrl(r.approvalUrl);
      await load(); // 动作回执后重探一次（GET 时机 = 挂载/动作后，不做时刻轮询）
    } catch (e) {
      // 失败不弹全局 toast 抢焦点：把错误落到对应步骤行（可见错误行），重探刷新状态
      setStepError({ step, message: typeof e === "string" ? e : String(e) });
      await load();
    } finally {
      setBusyStep(null);
    }
  };

  const stateOf = (id: string): TsStepState | undefined => probe?.states.find((s) => s.id === id);
  // §C3：头部地址门控——只有校验通过（Verified）才算可用地址；未验证/失败一律不展示
  const reachVerified = probe?.reach?.state === "verified";

  return (
    <div data-testid="ts-wizard">
      <div className="flex items-center justify-between gap-2">
        <p className="text-sm font-semibold">{t("settings.remote.tsWizard.title")}</p>
        <Button variant="ghost" size="sm" onClick={() => void load()}>
          {t("settings.remote.tsWizard.refresh")}
        </Button>
      </div>
      {/* Windows 行弱提示（I-3：验证位不得大于证据）：2026-10-07 真机探测**从第 6 步
          （shields_up）开始**——detect/download/install(UAC)/login 四步的**流程**没被
          端到端跑过，故提示**不得整行撤下**，而要收窄成精确范围并**点名**那几步
          （清单来自后端 windowsUnverifiedSteps，随步骤表派生，不写死文案）。 */}
      {probe?.platform === "windows" && !probe.windowsVerified && (
        <p
          data-testid="ts-windows-unverified"
          className="mt-1 rounded-lg bg-amber-50 px-2.5 py-1.5 text-xs text-amber-600 dark:bg-amber-500/10 dark:text-amber-400"
        >
          {t("settings.remote.tsWizard.windowsUnverified", {
            steps: (probe.windowsUnverifiedSteps ?? [])
              .map((id) => (STEP_KEY[id] ? t(STEP_KEY[id]) : id))
              .join(" → "),
          })}
        </p>
      )}
      {/* B2：写路径未实机验证弱提示（与 Windows 行同款做法）——macOS 的
          `funnel --bg` / `reset` / 批准链接抓取从未在本机跑过，不得伪装成已验证；
          Windows 已实测三条写路径，后端位为 true 时本条自动消失。
          M-4：文案**逐平台**取（macOS 那份"只读探测已实测"只在 macOS 成立——Linux/Other
          上什么都不曾探测过，套用 macOS 的表述就是张冠李戴的谎报）。 */}
      {probe && !probe.writePathVerified && (
        <p
          data-testid="ts-writepath-unverified"
          className="mt-1 rounded-lg bg-amber-50 px-2.5 py-1.5 text-xs text-amber-600 dark:bg-amber-500/10 dark:text-amber-400"
        >
          {probe.platform === "mac"
            ? t("settings.remote.tsWizard.writePathUnverifiedMac")
            : t("settings.remote.tsWizard.writePathUnverifiedOther")}
        </p>
      )}
      {loadError !== null && (
        <p data-testid="ts-load-error" className="mt-1 text-xs text-rose-500">
          {t("settings.remote.tsWizard.loadFailed")}: {loadError}
        </p>
      )}
      {probe === null && loadError === null && (
        <p className="text-muted-foreground mt-2 animate-pulse text-xs">…</p>
      )}
      {/* §C3：看板地址只在通道运行且校验通过（Verified）时展示——未验证不得当可用
          地址让用户干等（失败/待验的呈现走 verify 步行的「尚未生效 + 重试」） */}
      {probe?.running && probe.boardUrl && reachVerified && (
        <p className="text-muted-foreground mt-1 text-xs">
          <span className="mr-1 inline-block h-2 w-2 rounded-full bg-emerald-500 align-middle" />
          {t("settings.remote.tsWizard.runningBadge")} ·{" "}
          {t("settings.remote.tsWizard.boardUrlLabel")}:{" "}
          <code className="bg-muted rounded-md px-1.5 py-0.5 font-mono text-[12px]">
            {probe.boardUrl}
          </code>
        </p>
      )}
      {/* W-A：**开机恢复窗口**（后端重连中）——独立语义块，先于其他提示判定：
          既不是故障（不显示 verify 行的「尚未生效」），也不是「域名生效中」（重启后
          DNS 记录不撤销，压根不用等发布）。判据是后端下发的 reach.state=recovering，
          与 probe.running 无关（恢复窗口里快照就是 running=false）。 */}
      {probe?.reach?.state === "recovering" && (
        <p
          data-testid="ts-recovering"
          className="mt-1 flex flex-wrap items-center gap-2 rounded-lg bg-amber-50 px-2.5 py-1.5 text-xs text-amber-600 dark:bg-amber-500/10 dark:text-amber-400"
        >
          {/* M3：与线稿状态二 354 行同形（琥珀 badge + 说明） */}
          <AmberBadge>{t("settings.remote.tsWizard.tsRecoveringBadge")}</AmberBadge>
          {t("settings.remote.tsWizard.tsRecoveringHint")}
        </p>
      )}
      {/* A2/W-B：通道在跑但校验未通过＝**正常现象**，必须给出预期时长——实测**三档**
          时长差异极大（首开 ≈5–6 分钟 / reset 后重开 ≈30–49 秒 / 重启恢复 1–2 分钟且
          成因是后端重连）。档位由后端给出（reach.state=record_pending + republish），
          前端**不猜成因**；recovering 走上块，不在本块重复。Failed 另有具体成因
          （在 verify 行点名），此处不重复。 */}
      {probe?.running && !reachVerified && probe.reach?.state === "record_pending" && (
        <p
          data-testid={probe.reach.republish ? "ts-republish" : "ts-pending"}
          className="mt-1 flex flex-wrap items-center gap-2 rounded-lg bg-amber-50 px-2.5 py-1.5 text-xs text-amber-600 dark:bg-amber-500/10 dark:text-amber-400"
        >
          {/* M3：线稿状态二把"发布中"（355 行）画成带 badge 的琥珀行，而"记录尚未发布"
              （357 行）是**不带 badge** 的普通行——实现照线稿分档，不一律加徽标 */}
          {probe.reach.republish && (
            <AmberBadge>{t("settings.remote.tsWizard.tsRepublishBadge")}</AmberBadge>
          )}
          {probe.reach.republish
            ? t("settings.remote.tsWizard.tsRepublishHint")
            : t("settings.remote.tsWizard.tsPendingHint")}
        </p>
      )}
      {/* 未验证 / 在验（还没跑过校验，档位无从判定）：给首开档口径（自带"此前开通过
          1 分钟内"的下界，不谎称）；第一轮校验（≤1 个轮询窗）后自动切到准确档位 */}
      {probe?.running &&
        !reachVerified &&
        (probe.reach?.state === "unverified" || probe.reach?.state === "verifying") && (
          <p
            data-testid="ts-pending"
            className="mt-1 rounded-lg bg-amber-50 px-2.5 py-1.5 text-xs text-amber-600 dark:bg-amber-500/10 dark:text-amber-400"
          >
            {t("settings.remote.tsWizard.tsPendingHint")}
          </p>
        )}
      <div className="mt-1">
        {(probe?.steps ?? []).map((step: TsWizardStep) => {
          const st = stateOf(step.id);
          const done = st?.done ?? false;
          const busy = busyStep === step.id;
          const blocked = st?.blockedReason ?? null;
          const actionText = step.needsHuman && step.humanActionKey ? t(step.humanActionKey) : "";
          return (
            <div
              key={step.id}
              data-step={step.id}
              data-done={done ? "true" : "false"}
              className="flex items-start gap-2.5 py-2"
            >
              {/* 三态点：已完成绿 / 在途蓝脉冲 / 待做灰（卡住以文字呈现，不假装转圈） */}
              <span
                data-dot
                className={cn(
                  "mt-1.5 inline-block h-2 w-2 flex-none rounded-full",
                  done ? "bg-emerald-500" : busy ? "animate-pulse bg-blue-500" : "bg-gray-300"
                )}
              />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className={cn("text-[13px]", done && "text-muted-foreground")}>
                    {STEP_KEY[step.id] ? t(STEP_KEY[step.id]) : step.id}
                  </span>
                  <span className={SETTINGS_NOTE}>
                    {done
                      ? t("settings.remote.tsWizard.stateDone")
                      : busy
                        ? t("settings.remote.tsWizard.stateActive")
                        : t("settings.remote.tsWizard.statePending")}
                  </span>
                  {RUNNABLE.has(step.id) && !done && (
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-6 px-2 text-xs"
                      disabled={busyStep !== null}
                      onClick={() => void run(step.id)}
                    >
                      {t("settings.remote.tsWizard.runStep")}
                    </Button>
                  )}
                  {/* login：MAM 不代登录——只递授权链接（authUrl 由 probe/run_step 透出） */}
                  {step.id === "login" && !done && probe?.authUrl && (
                    <a
                      data-auth
                      data-testid="ts-auth-link"
                      href={probe.authUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="text-xs text-blue-500 underline"
                    >
                      {t("settings.remote.tsWizard.openAuthUrl")}
                    </a>
                  )}
                  {/* verify：§C3 专属重试钮（未验/失败都可重触发校验；后端含自愈） */}
                  {step.id === "verify" && !done && (
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-6 px-2 text-xs"
                      data-testid="ts-verify-retry"
                      disabled={busyStep !== null}
                      onClick={() => void run("verify")}
                    >
                      {t("settings.remote.tsWizard.retry")}
                    </Button>
                  )}
                </div>
                {/* 需要人的步骤：动作文案突出显示（后端 humanActionKey 下发，前端只翻译）。
                    A1：可选人工步（humanOptional，目前只有 funnel 批准）额外挂一条弱提示
                    ——实测两平台各执一端（Windows 1.102.4 零批准链接、零点击；macOS 首开
                    确有一次），故**不得**把批准渲染成"等你点批准"的必经步骤，也不得据此
                    阻塞；链接真出现时由下方 approvalUrl 行给出。 */}
                {actionText && (
                  <p className="mt-0.5 text-xs font-medium text-amber-600 dark:text-amber-400">
                    {actionText}
                  </p>
                )}
                {step.needsHuman && step.humanOptional && (
                  <p
                    data-testid="ts-optional-step"
                    className="text-muted-foreground mt-0.5 text-[11px]"
                  >
                    {t("settings.remote.tsWizard.maybeNeeded")}
                  </p>
                )}
                {/* 卡住必须点名原因，不许只转圈（verify 行有专属「尚未生效」呈现，见下）。
                    **M4（2026-10-07 评审 Minor）**：色/语义档位随后端判据走——`amber`
                    是中间态（恢复窗口「正在进行、无需操作」，线稿 82-84 明写「既不是故障
                    （rose）也不是正常完成」）：不涂红、也不加「卡住：」前缀（措辞本就如实，
                    只是档位不对），直接给成因与"稍候自动复评"。缺字段按 rose（fail-safe）。 */}
                {blocked && step.id !== "verify" && (
                  <p
                    data-blocked
                    data-testid="ts-blocked"
                    data-blocked-tone={st?.blockedTone ?? "rose"}
                    className={cn(
                      "mt-0.5 text-xs",
                      (st?.blockedTone ?? "rose") === "amber"
                        ? "text-amber-600 dark:text-amber-400"
                        : "text-rose-500"
                    )}
                  >
                    {(st?.blockedTone ?? "rose") === "rose" &&
                      `${t("settings.remote.tsWizard.blockedTitle")} `}
                    {blocked}
                  </p>
                )}
                {/* verify §C3：校验失败 = 固定地址尚未生效——点名口径 + reason，
                    不显示成可用地址让用户干等 */}
                {step.id === "verify" && blocked && (
                  <p data-testid="ts-not-live" className="mt-0.5 text-xs text-rose-500">
                    {t("settings.remote.tsWizard.tsNotLive")}：{blocked}
                  </p>
                )}
                {/* 动作失败落行内错误位（Task 6 移交指针：不再吞错误零反馈） */}
                {stepError?.step === step.id && (
                  <p data-testid="ts-step-error" className="mt-0.5 text-xs text-rose-500">
                    {t("settings.remote.tsWizard.actionFailed")} {stepError.message}
                  </p>
                )}
              </div>
            </div>
          );
        })}
      </div>
      {approvalUrl && (
        <p data-testid="ts-approval" className="mt-1 text-xs">
          <span className="text-amber-600 dark:text-amber-400">
            {t("settings.remote.tsWizard.approvalHint")}
          </span>{" "}
          <a
            href={approvalUrl}
            target="_blank"
            rel="noreferrer"
            className="text-blue-500 underline"
          >
            {approvalUrl}
          </a>
        </p>
      )}
    </div>
  );
}
