import { cn } from "@/lib/utils";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect, useState, type ReactNode } from "react";

type WindowFrameProps = {
  titleBar: ReactNode;
  children: ReactNode;
  className?: string;
  contentClassName?: string;
};

export function WindowFrame({ titleBar, children, className, contentClassName }: WindowFrameProps) {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWebviewWindow();

    appWindow.isMaximized().then(setIsMaximized);

    const unlistenResize = appWindow.onResized(async () => {
      const maximized = await appWindow.isMaximized();
      setIsMaximized(maximized);
    });

    return () => {
      unlistenResize.then((fn) => fn());
    };
  }, []);

  // 主题由 theme-provider 的全局 store 管理（无 Provider，模块加载即应用）
  return (
    <div
      className={cn(
        "bg-background flex h-screen w-screen flex-col overflow-hidden",
        isMaximized ? "" : "border-border rounded-lg border",
        className
      )}
    >
      {titleBar}
      <main className={contentClassName}>{children}</main>
    </div>
  );
}
