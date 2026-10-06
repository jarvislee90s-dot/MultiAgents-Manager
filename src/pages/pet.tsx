// src/pages/pet.tsx — 宠物窗口路由页：应用显隐/置顶后渲染桌宠（spec §4.1/§4.5）
import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { Toaster } from "@/components/ui/sonner";
import { FoxbellPet } from "@/components/pet/FoxbellPet";
import { loadConfig, loadVisible, saveVisible, subscribeConfig } from "@/components/pet/petConfig";
import { invoke } from "@tauri-apps/api/core";

export default function PetPage() {
  useEffect(() => {
    // 诊断（决策三输入）：这一行回答「本次走的是哪条路」——`visible` 决定是否 show，
    // `alwaysOnTop` 决定置顶是否被设，`hash` 证明这个 WebView 确实被判成了宠物窗口
    // （`main.tsx` 按 `#/pet` 分流；分错了会静默回落首页 ⇒ 本 effect 压根不跑）。
    const requestedVisible = loadVisible();
    const cfg = loadConfig();
    console.info("[pet] mount", {
      hash: window.location.hash,
      visible: requestedVisible,
      alwaysOnTop: cfg.alwaysOnTop,
    });
    const apply = async () => {
      try {
        await getCurrentWindow().setAlwaysOnTop(cfg.alwaysOnTop);
        if (requestedVisible) await getCurrentWindow().show();
      } catch (e) {
        // 诊断：原先这里是**空 catch**（「浏览器预览：忽略」）⇒ ACL 拒绝 / 窗口 API 失败
        // 全被吞掉。它不是致命路径（下面还有 `set_pet_visible` 兜底），但必须留痕。
        console.error("[pet] apply() 失败（setAlwaysOnTop / show）", e);
      }
    };
    apply();
    // 托盘/主窗口切换显隐后同步本地状态（spec §10.2）
    const un1 = listen<{ visible: boolean }>("pet-visibility-changed", (e) => {
      console.info("[pet] pet-visibility-changed →", e.payload.visible);
      saveVisible(e.payload.visible);
    }).catch(() => Promise.resolve(() => {}));
    const un2 = subscribeConfig(() => {
      const cfg2 = loadConfig();
      getCurrentWindow()
        .setAlwaysOnTop(cfg2.alwaysOnTop)
        .catch((e) => console.error("[pet] setAlwaysOnTop（配置变更）失败", e));
    });
    // 兜底：窗口存活但从未显式 show（如首次开启）。
    // 诊断（**最要紧的一行**）：这个调用是宠物窗口**唯一**的显示路径，而原先的
    // `.catch(() => {})` 把它的一切失败都吞掉了 ——「宠物不显示」因此**永远无迹可查**。
    // 失败必须留痕，且要带上「发出去的 visible 值」，否则分不清是「没发」还是「发了没生效」。
    invoke("set_pet_visible", { visible: requestedVisible }).catch((e) => {
      console.error("[pet] set_pet_visible 失败", { requested: requestedVisible, error: e });
    });
    return () => {
      un1.then((f) => f());
      un2();
    };
  }, []);
  return (
    <>
      <FoxbellPet />
      {/* P2-7（issue #34）：宠物窗口此前未挂 Toaster，useSessionJump 的 M3 兜底
          提示与跳转失败提示在此窗口静默丢弃；顶部居中避免被桌宠本体遮挡 */}
      <Toaster position="top-center" />
    </>
  );
}
