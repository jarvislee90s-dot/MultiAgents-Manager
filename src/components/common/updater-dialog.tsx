// 升级提醒弹窗（prerelease 渠道，2026-10-07 设计定案）：
// - 三按钮语义：立即升级 / 忽略本版本（DB KV 持久，直到新版本出现）/ 稍后（仅本次）
// - 正文以 release 页 body 为准，react-markdown 渲染 + 限高内滚（长 note 不撑爆弹窗）
// - 预发布角标；「在 GitHub 查看」跳具体 tag 页（html_url，非 /releases/latest）
// - 自动模式（home 挂载）每次启动检查一次：可用且未被忽略才自动弹；
//   手动模式（About 挂载）由按钮触发检查，出结果即弹
// - 图片弱网降级：加载失败显示 alt 文案占位，不露破图
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import { useUpdaterStore } from "@/stores/updaterStore";
import type { UpdateProgress } from "@/lib/updater";

interface UpdaterDialogProps {
  /** 手动模式（About 挂载）：不自动检查，由手动检查结果驱动弹窗 */
  manualCheck?: boolean;
}

/** note 内嵌图弱网降级：加载失败显示 alt 文案占位（raw.githubusercontent 时常不可达） */
function NotesImage({ src, alt }: { src?: string; alt?: string }) {
  const [failed, setFailed] = useState(false);
  if (failed || !src) {
    return (
      <span className="text-muted-foreground my-1 block rounded-md border border-dashed px-2 py-1 text-xs">
        🖼 {alt || "图片未能加载"}
      </span>
    );
  }
  return (
    <img
      src={src}
      alt={alt ?? ""}
      loading="lazy"
      onError={() => setFailed(true)}
      className="my-1 max-w-full rounded-md border"
    />
  );
}

/** release note 的 Markdown 元素样式（无 typography 插件，显式覆盖） */
const markdownComponents = {
  h1: (p: React.ComponentProps<"h1">) => (
    <h1 {...p} className="mb-1.5 mt-3 text-sm font-semibold first:mt-0" />
  ),
  h2: (p: React.ComponentProps<"h2">) => (
    <h2 {...p} className="mb-1.5 mt-3 text-sm font-semibold first:mt-0" />
  ),
  h3: (p: React.ComponentProps<"h3">) => (
    <h3 {...p} className="mb-1 mt-2 text-[13px] font-semibold first:mt-0" />
  ),
  p: (p: React.ComponentProps<"p">) => <p {...p} className="my-1.5 leading-relaxed" />,
  ul: (p: React.ComponentProps<"ul">) => <ul {...p} className="my-1.5 list-disc pl-5" />,
  ol: (p: React.ComponentProps<"ol">) => <ol {...p} className="my-1.5 list-decimal pl-5" />,
  li: (p: React.ComponentProps<"li">) => <li {...p} className="my-0.5" />,
  a: (p: React.ComponentProps<"a">) => (
    <a {...p} className="underline underline-offset-2" target="_blank" rel="noreferrer" />
  ),
  blockquote: (p: React.ComponentProps<"blockquote">) => (
    <blockquote {...p} className="text-muted-foreground my-1.5 border-l-2 pl-3" />
  ),
  code: (p: React.ComponentProps<"code">) => (
    <code
      {...p}
      className="bg-accent rounded border px-1 font-mono text-[11px] break-all"
    />
  ),
  hr: () => <hr className="my-2.5" />,
  table: (p: React.ComponentProps<"table">) => (
    <table {...p} className="my-2 w-full border-collapse text-xs" />
  ),
  th: (p: React.ComponentProps<"th">) => (
    <th {...p} className="bg-accent border px-1.5 py-1 text-left" />
  ),
  td: (p: React.ComponentProps<"td">) => (
    <td {...p} className="border px-1.5 py-1 align-top" />
  ),
  img: ({ src, alt }: { src?: string; alt?: string }) => (
    <NotesImage src={src} alt={alt} />
  ),
};

export function UpdaterDialog({ manualCheck = false }: UpdaterDialogProps) {
  const result = useUpdaterStore((s) => s.result);
  const checking = useUpdaterStore((s) => s.checking);
  const downloading = useUpdaterStore((s) => s.downloading);
  const progress = useUpdaterStore((s) => s.progress);
  const dialogOpen = useUpdaterStore((s) => s.dialogOpen);
  const skippedVersion = useUpdaterStore((s) => s.skippedVersion);
  const skippedLoaded = useUpdaterStore((s) => s.skippedLoaded);
  const checkUpdate = useUpdaterStore((s) => s.checkUpdate);
  const installUpdate = useUpdaterStore((s) => s.installUpdate);
  const openDialog = useUpdaterStore((s) => s.openDialog);
  const closeDialog = useUpdaterStore((s) => s.closeDialog);
  const skipVersion = useUpdaterStore((s) => s.skipVersion);
  const loadSkippedVersion = useUpdaterStore((s) => s.loadSkippedVersion);
  const { t } = useTranslation();

  const update = result?.status === "available" ? result.update : null;

  // 自动模式：启动即查（含已忽略版本载入），保持一次启动一次远端检查；
  // autoCheckDone 门栓兜住 dev StrictMode 双挂载（评审 M4）
  useEffect(() => {
    if (!manualCheck && !useUpdaterStore.getState().autoCheckDone) {
      useUpdaterStore.setState({ autoCheckDone: true });
      void loadSkippedVersion();
      void checkUpdate();
    }
  }, [manualCheck, checkUpdate, loadSkippedVersion]);

  // 自动弹窗判定：可用 && 已忽略版本载入完毕 && 未被忽略 → 弹一次（本进程不再弹）
  const autoHandledRef = useRef(false);
  useEffect(() => {
    if (manualCheck || autoHandledRef.current) return;
    if (result?.status !== "available" || !skippedLoaded) return;
    autoHandledRef.current = true;
    if (result.update.version !== skippedVersion) {
      openDialog();
    }
  }, [manualCheck, result, skippedLoaded, skippedVersion, openDialog]);

  // 手动模式：检查结束出结果即弹（无更新时由 useManualUpdateCheck 显示行内提示）。
  // checking 再变 true 时复位一次性标记：同一会话内再次手动检查发现新版本仍要弹
  // （评审 I5：ref 不复位会让第二次检查完全无反馈）
  const manualCompletedRef = useRef(false);
  useEffect(() => {
    if (!manualCheck) return;
    if (checking) {
      manualCompletedRef.current = false;
      return;
    }
    if (manualCompletedRef.current || !result) return;
    manualCompletedRef.current = true;
    if (result.status === "available") openDialog();
  }, [manualCheck, checking, result, openDialog]);

  // note 限高滚动提示：内容超出可视高度才显示
  const notesRef = useRef<HTMLDivElement>(null);
  const [notesScrollable, setNotesScrollable] = useState(false);
  useLayoutEffect(() => {
    const el = notesRef.current;
    if (el) setNotesScrollable(el.scrollHeight > el.clientHeight + 4);
  }, [update?.notes, dialogOpen]);

  // 评审 I2：安装失败必须给用户可操作反馈（如「此构建未启用应用内升级」），
  // 而不是按钮灰一下又亮回来
  const handleInstall = async () => {
    const r = await installUpdate();
    if (!r.ok && r.message) {
      toast.error(t("updater.installFailed", { message: r.message }));
    }
  };

  /** 稍后 / 关闭 = 仅本次忽略（下次启动再弹） */
  const handleLater = () => closeDialog();

  const handleSkip = () => {
    if (update) {
      void skipVersion(update.version);
      toast.info(t("updater.skippedToast", { version: update.version }));
    }
    closeDialog();
  };

  const handleOpenGithub = () => {
    if (update) void openUrl(update.htmlUrl);
  };

  const getProgressPercentage = (p: UpdateProgress | null) => {
    if (!p || p.event === "Started") return 0;
    const { downloaded, contentLength } = p.data || {};
    if (!contentLength) return 0;
    if (p.event === "Finished") return 100;
    return Math.round(((downloaded ?? 0) / contentLength) * 100);
  };

  return (
    <Dialog open={dialogOpen} onOpenChange={(open) => !open && closeDialog()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {downloading ? t("updater.downloading") : t("updater.updateAvailable")}
            {!downloading && update?.prerelease && (
              <span className="rounded-full border border-amber-500 px-2 py-px text-[11px] font-medium text-amber-500">
                {t("updater.prerelease")}
              </span>
            )}
          </DialogTitle>
          <DialogDescription asChild>
            <div className="space-y-2">
              {downloading ? (
                <div className="space-y-2">
                  <p>{t("updater.installingVersion", { version: update?.version })}</p>
                  <Progress value={getProgressPercentage(progress)} />
                </div>
              ) : (
                <>
                  <p>
                    {t("updater.versionAvailable", {
                      version: update?.version,
                      currentVersion:
                        result?.status === "available" ? result.currentVersion : "",
                    })}
                    {update?.publishedAt && (
                      <span className="text-muted-foreground">
                        {" · "}
                        {t("updater.publishedOn", { date: update.publishedAt.slice(0, 10) })}
                      </span>
                    )}
                  </p>
                  <button
                    onClick={handleOpenGithub}
                    className="text-muted-foreground underline underline-offset-2 transition-colors hover:text-foreground"
                  >
                    {t("updater.viewOnGithub")}
                  </button>
                  {update?.notes && (
                    <>
                      <p className="mt-2 font-semibold">{t("updater.releaseNotes")}</p>
                      <div
                        ref={notesRef}
                        className="bg-muted max-h-56 overflow-y-auto rounded-md p-3 text-sm leading-relaxed"
                      >
                        <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
                          {update.notes}
                        </ReactMarkdown>
                      </div>
                      {notesScrollable && (
                        <p className="text-muted-foreground text-center text-[11px]">
                          {t("updater.scrollHint")}
                        </p>
                      )}
                    </>
                  )}
                </>
              )}
            </div>
          </DialogDescription>
        </DialogHeader>
        {!downloading && (
          <DialogFooter>
            <Button variant="ghost" onClick={handleSkip}>
              {t("updater.skipVersion")}
            </Button>
            <Button variant="outline" onClick={handleLater}>
              {t("updater.later")}
            </Button>
            <Button onClick={handleInstall}>{t("updater.installNow")}</Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** About 页手动检查：出结果前禁用按钮/出结果后给行内「已是最新」或错误 toast */
export function useManualUpdateCheck() {
  const checkUpdate = useUpdaterStore((s) => s.checkUpdate);
  const checking = useUpdaterStore((s) => s.checking);
  const result = useUpdaterStore((s) => s.result);
  const [showNoUpdate, setShowNoUpdate] = useState(false);
  const { t } = useTranslation();

  const handleCheckUpdate = async () => {
    setShowNoUpdate(false);
    const r = await checkUpdate();

    if (r.status === "up-to-date") {
      setShowNoUpdate(true);
      return;
    }
    if (r.status === "error") {
      toast.error(t("updater.checkFailed"));
    }
    // available → UpdaterDialog（manualCheck 模式）自动弹窗
  };

  return {
    checkUpdate: handleCheckUpdate,
    checking,
    hasUpdate: result?.status === "available",
    showNoUpdate,
    dismissNoUpdate: () => setShowNoUpdate(false),
  };
}
