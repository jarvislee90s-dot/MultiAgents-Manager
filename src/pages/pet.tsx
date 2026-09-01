// src/pages/pet.tsx — 宠物窗口路由页：应用显隐/置顶后渲染桌宠（spec §4.1/§4.5）
import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { FoxbellPet } from "@/components/pet/FoxbellPet";
import { loadConfig, loadVisible, saveVisible, subscribeConfig } from "@/components/pet/petConfig";
import { invoke } from "@tauri-apps/api/core";

export default function PetPage() {
  useEffect(() => {
    const apply = async () => {
      const cfg = loadConfig();
      try {
        await getCurrentWindow().setAlwaysOnTop(cfg.alwaysOnTop);
        if (loadVisible()) await getCurrentWindow().show();
      } catch {
        // 浏览器预览：忽略
      }
    };
    apply();
    // 托盘/主窗口切换显隐后同步本地状态（spec §10.2）
    const un1 = listen<{ visible: boolean }>("pet-visibility-changed", (e) => {
      saveVisible(e.payload.visible);
    }).catch(() => Promise.resolve(() => {}));
    const un2 = subscribeConfig(() => {
      const cfg = loadConfig();
      getCurrentWindow().setAlwaysOnTop(cfg.alwaysOnTop).catch(() => {});
    });
    // 兜底：窗口存活但从未显式 show（如首次开启）
    invoke("set_pet_visible", { visible: loadVisible() }).catch(() => {});
    return () => {
      un1.then((f) => f());
      un2();
    };
  }, []);
  return <FoxbellPet />;
}
