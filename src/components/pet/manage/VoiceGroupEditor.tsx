// VoiceGroupEditor — 四分组音频编辑器（导入向导暂存模式 / 修改面板直写模式共用，spec §8.4-3/§10-3）
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import {
  AUDIO_EXTS,
  GROUPS,
  judgeVoiceTier,
  voiceRowProblem,
  type VoiceRow,
} from "../petValidation";

const GROUP_LABEL_KEY: Record<string, string> = {
  general: "pet.import.groupGeneral",
  approval: "pet.import.groupApproval",
  done: "pet.import.groupDone",
  error: "pet.import.groupError",
};

export function VoiceGroupEditor(props: {
  rows: VoiceRow[];
  onAdd: (group: string, paths: string[]) => void | Promise<void>;
  onRemove: (rel: string) => void | Promise<void>;
  busy?: boolean;
}) {
  const { t } = useTranslation();
  const totalBytes = props.rows.reduce((s, r) => s + r.sizeBytes, 0);
  const valid = props.rows.filter((r) => !voiceRowProblem(r));
  const judge = judgeVoiceTier(
    valid.map((r) => ({ rel: r.file, size: r.sizeBytes, durationMs: r.durationMs }))
  );
  const missing = GROUPS.filter((g) => judge.coverage[g] === 0);

  const add = async (group: string) => {
    const paths = (await open({
      multiple: true,
      filters: [{ name: "Audio", extensions: AUDIO_EXTS }],
    })) as string[] | string | null;
    if (!paths) return;
    await props.onAdd(group, Array.isArray(paths) ? paths : [paths]);
  };

  return (
    <div className="space-y-3" data-testid="voice-group-editor">
      {GROUPS.map((g) => {
        const list = props.rows.filter((r) => r.group === g);
        return (
          <div key={g} data-testid={`voice-group-${g}`} title={t(GROUP_LABEL_KEY[g])}>
            <div className="mb-1 flex items-center justify-between">
              <span className="text-sm font-medium">{g}</span>
              <Button
                size="sm"
                variant="outline"
                disabled={props.busy}
                data-testid={`voice-add-${g}`}
                onClick={() => void add(g)}
              >
                {t("pet.import.addAudio")}
              </Button>
            </div>
            {list.map((r) => {
              const problem = voiceRowProblem(r);
              return (
                <div
                  key={r.file}
                  data-testid={`voice-row-${r.file}`}
                  className="text-muted-foreground flex items-center justify-between py-0.5 text-xs"
                >
                  <span className="max-w-[60%] truncate">
                    {r.name}
                    {r.durationMs !== null && (
                      <span className="ml-1">
                        ({t("pet.import.duration", { ms: (r.durationMs / 1000).toFixed(1) })})
                      </span>
                    )}
                  </span>
                  <span className="flex items-center gap-2">
                    {problem && (
                      <span className="bg-destructive/15 text-destructive rounded px-1">
                        {t(`pet.import.problems.${problem}`)}
                      </span>
                    )}
                    <button
                      data-testid={`voice-remove-${r.file}`}
                      className="underline-offset-2 hover:underline"
                      onClick={() => void props.onRemove(r.file)}
                    >
                      {t("pet.import.remove")}
                    </button>
                  </span>
                </div>
              );
            })}
          </div>
        );
      })}
      <div data-testid="voice-coverage" className="text-xs">
        {missing.length === 0 ? (
          <span className="text-primary">{t("pet.import.coverageOk")}</span>
        ) : (
          <span className="text-muted-foreground">
            {t("pet.import.coverageMissing", { groups: missing.join(", ") })}
          </span>
        )}
        <span className="ml-2">
          {t("pet.import.totalSize", { size: `${(totalBytes / 1024 / 1024).toFixed(1)}MB` })}
        </span>
        {totalBytes > 30 * 1024 * 1024 && (
          <span className="text-destructive ml-1">{t("pet.import.tooLargeWarn")}</span>
        )}
      </div>
    </div>
  );
}
