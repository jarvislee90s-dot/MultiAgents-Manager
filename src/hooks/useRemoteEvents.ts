// 桌面全局远程事件（M4 T2）：配对请求系统通知（点击直达设置页——动作类型经
// useNotification.ts 的既有全局注册，本 hook 不得二次注册）/ 隧道地址与守护
// 失败通知 / 托盘触发的地址复制与开关失败提示。main.tsx AppWrapper 挂载一次。
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  sendNotification,
  isPermissionGranted,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { toast } from "sonner";
import { useAppTranslation } from "@/hooks/use-app-translation";

export function useRemoteEvents() {
  const { t } = useAppTranslation();
  useEffect(() => {
    // 通知浮窗/宠物窗口不订阅：各自 WebView 重复弹系统通知与 toast，主窗口是唯一出口
    if (window.location.hash === "#/notification" || window.location.hash === "#/pet") return;
    const unlisten: Array<() => void> = [];
    void (async () => {
      // 系统通知统一出口：权限未授先请求；动作类型默认 open-pairing（点击直达设置页，
      // 分发在 useNotification.ts 的全局 onAction 单点）
      const notify = async (title: string, body: string, actionTypeId = "open-pairing") => {
        let ok = await isPermissionGranted();
        if (!ok) ok = (await requestPermission()) === "granted";
        if (ok) sendNotification({ title, body, actionTypeId });
      };
      unlisten.push(
        await listen<{ name: string; ip: string }>("remote-pair-request", async (e) => {
          // spec T2a：桌面弹通知 + 点击直达配对面板（设置页）
          await notify(
            t("settings.remote.pairNotifyTitle"),
            t("settings.remote.pairNotifyBody", {
              name: e.payload.name || t("settings.remote.pendingUnknown"),
              ip: e.payload.ip,
            })
          );
          toast.info(t("settings.remote.pairNotifyToast"));
        })
      );
      unlisten.push(
        await listen<{ url: string }>("remote-tunnel-address", (e) => {
          toast.success(t("settings.remote.tunnelReadyToast", { url: e.payload.url }));
        })
      );
      unlisten.push(
        await listen<{ error: string }>("remote-tunnel-error", async (e) => {
          // spec T1b：守护放弃时桌面通知报错（toast 在窗口隐藏时不可见——双通道）
          await notify(t("settings.remote.tunnelErrorTitle"), e.payload.error);
          toast.error(t("settings.remote.tunnelErrorToast", { error: e.payload.error }));
        })
      );
      unlisten.push(
        await listen<{ url: string }>("remote-copy-addr", async (e) => {
          // 托盘菜单「复制远程地址」：剪贴板写入在本窗口完成（托盘进程无剪贴板上下文）
          await navigator.clipboard.writeText(e.payload.url);
          toast.success(t("settings.remote.addrCopiedToast"));
        })
      );
      unlisten.push(
        await listen<{ error: string }>("remote-toggle-failed", async (e) => {
          // spec T4：托盘开关失败「桌面通知报错」——toast + 系统通知双通道
          await notify(t("settings.remote.toggleFailedTitle"), e.payload.error);
          toast.error(t("settings.remote.toggleFailedToast", { error: e.payload.error }));
        })
      );
    })();
    return () => unlisten.forEach((f) => f());
  }, [t]);
}
