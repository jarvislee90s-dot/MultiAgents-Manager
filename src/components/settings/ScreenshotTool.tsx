import { useState } from "react";
import { useTranslation } from "react-i18next";
import { captureScreenshot, listScreenshots } from "@/lib/screenshot";
import { Button } from "@/components/ui/button";
import { Camera, Image } from "lucide-react";
import { toast } from "sonner";

export function ScreenshotTool() {
  const { t } = useTranslation();
  const [capturing, setCapturing] = useState(false);
  const [screenshots, setScreenshots] = useState<string[]>([]);

  const handleCapture = async () => {
    setCapturing(true);
    try {
      const result = await captureScreenshot();
      if (result.success && result.path) {
        toast.success(t("screenshot.saved", { path: result.path }));
        // 刷新截图列表
        const list = await listScreenshots();
        setScreenshots(list);
      } else {
        toast.error(t("screenshot.failed", { error: result.error || t("common.unknown") }));
      }
    } catch (e) {
      toast.error(t("screenshot.failed", { error: e }));
    } finally {
      setCapturing(false);
    }
  };

  const loadScreenshots = async () => {
    try {
      const list = await listScreenshots();
      setScreenshots(list);
    } catch (e) {
      console.error("Failed to load screenshots:", e);
    }
  };

  return (
    <div className="space-y-3 rounded border p-3">
      <h3 className="flex items-center gap-2 text-sm font-semibold">
        <Camera className="h-4 w-4" />
        {t("screenshot.title")}
      </h3>

      <div className="flex gap-2">
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-[10px]"
          onClick={handleCapture}
          disabled={capturing}
        >
          <Camera className={`mr-1 h-3 w-3 ${capturing ? "animate-spin" : ""}`} />
          {capturing ? t("screenshot.capturing") : t("screenshot.capture")}
        </Button>
        <Button size="sm" variant="ghost" className="h-7 text-[10px]" onClick={loadScreenshots}>
          <Image className="mr-1 h-3 w-3" />
          {t("screenshot.refreshList")}
        </Button>
      </div>

      {screenshots.length > 0 && (
        <div className="space-y-1">
          <h4 className="text-muted-foreground text-xs font-medium">{t("screenshot.recent")}</h4>
          <div className="space-y-1">
            {screenshots.slice(0, 5).map((path) => (
              <div key={path} className="flex items-center gap-2 text-xs">
                <Image className="text-muted-foreground h-3 w-3" />
                <span className="truncate text-[10px]">{path}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
