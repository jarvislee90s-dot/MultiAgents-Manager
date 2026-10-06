// 导出落盘**唯一接缝**（计划② Task 10）：把「内容生成」与「落到哪里」解耦到一处。
//
// 主出口恒为 **Rust 落盘**（`exportSaveText` / `exportSaveBytes` → `~/.mam/exports/`，回传落盘
// **绝对路径**）。**不走** `<a download>` / Blob 下载 / 对象 URL：wry 未注册 download handler 时
// 要么对 download 类导航直接 Cancel 且失败静默，要么把窗口导航到 blob URL（界面跑飞）；
// 也不走 `dialog.save` + plugin-fs（未安装、未声明、二级窗口不在 capability 白名单 → 必被 ACL 拒）。
// 定位复用既有 `revealDir`（白名单含 `~/.mam`），**不新写打开逻辑**。
// 剪贴板（`copyText` / `copyImage`）只作**副本**入口，**不得**作为导出的唯一出口。
//
// ## 三条外部约定（Rust 侧定的，前端只能遵守）
// ① **导出名必须以 `.csv` 结尾才会补 UTF-8 BOM**：Rust 侧 `with_bom_if_csv` 只认该后缀
//    （大小写不敏感）。补 BOM 是刻意的 Excel 兼容妥协——无 BOM 时中文 Windows 版按本地代码页
//    解码，中文列名乱码。**非 `.csv` 名一律拿不到 BOM**，且前端不得自行前置（上游哪天补了会出双 BOM）。
// ② **导出失败是自由文本**：两条导出命令是 `Result<_, String>`（静态中文文案，可能带系统错误原文），
//    **不是**用量命令的 `{ code, detail }` ⇒ 本层一律 `String(e)` 透传原因，**不得**喂给
//    `usageErrMsg`（那是结构化用量码专用，形状不兼容，翻必失真）。失败一律响亮地返回
//    `{ ok: false, reason: "failed", error }`，绝不吞错——吞错会让界面显示「导出成功」而磁盘上没有文件。
// ③ **空 / 纯空白 base64 → 0 字节文件 + `Ok`**：Rust 侧解码对空载荷解出空字节数组，落盘层只做
//    「解码后 ≤ 16 MB」检查 ⇒ 会写出一个 0 字节文件并返回成功路径。**触发条件**是 `bytes` 全空
//    （`base64Of` 得空串）或调用方直接传空/空白 base64；**当前不可达**：分享图由固定画布产出
//    （恒有非空尺寸），且本层只从 `bytes` 派生 base64。故此处只注明，**不改行为**——加一条
//    「空内容即拒」的前端分支会与 Rust 层的真实口径分叉，把不可达路径变成第三套规则。
import { exportSaveBytes, exportSaveText, revealDir } from "@/lib/api/usage";

/** 落盘结果：成功给**落盘绝对路径**（前端据此提示，并可交给 `openContainingDir` 定位）。 */
export type SaveOutcome =
  { ok: true; path: string } | { ok: false; reason: "failed"; error: string };

/** 剪贴板副本结果：目前只有剪贴板一条通路，`via` 是显式的（将来加通路必先改这里）。 */
export type CopyOutcome = { ok: true; via: "clipboard" } | { ok: false; error: string };

/**
 * 接缝的外部依赖（**可注入**：单测与组件测传假实现，绝不碰真 IPC、真剪贴板、真文件）。
 * 前三条**逐字对齐契约**的入参名与顺序（`name` / `content` / `base64` / `path`）——
 * 注意与公开 API 的 `saveText(content, filename)` 顺序相反，改这里前先看调用点。
 */
export interface SaveDeps {
  /** 契约 `exportSaveText(name, content)`；返回落盘绝对路径（**顺序即契约：`name` 在前**） */
  exportSaveText(name: string, content: string): Promise<string>;
  /** 契约 `exportSaveBytes(name, base64)`；二进制以 base64 **字符串**过线（number 数组会放大数倍体积） */
  exportSaveBytes(name: string, base64: string): Promise<string>;
  /** 既有 `revealDir(path)`：传**文件路径本身**（Rust 侧对文件是「在文件管理器里选中该文件」） */
  revealDir(path: string): Promise<void>;
  /** `navigator.clipboard.writeText`（仓内已有先例） */
  clipboardWriteText(text: string): Promise<void>;
  /**
   * `ClipboardItem` + `navigator.clipboard.write`（**best-effort**：全仓零先例，
   * 且 secure-context 前提未验证）。失败必须**如实上抛**，由 `copyImage` 转成自由文本原因。
   */
  clipboardWriteImage(blob: Blob): Promise<void>;
}

/**
 * 生产依赖：三条契约命令**原样**接上（同一份包装，不复制逻辑）；两个剪贴板原语取用时才求值
 * （`ClipboardItem` 在非安全上下文/老 WebView 里可能不存在——留到调用时抛，由 `copyImage` 如实报错）。
 */
export function defaultSaveDeps(): SaveDeps {
  return {
    exportSaveText,
    exportSaveBytes,
    revealDir,
    clipboardWriteText: (text) => navigator.clipboard.writeText(text),
    clipboardWriteImage: (blob) =>
      navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]),
  };
}

/** 分块大小（字节）：`String.fromCharCode` 的实参上限在分享图量级会 `RangeError`，故分块喂。 */
const BASE64_CHUNK = 0x8000;

/**
 * 字节 → base64 **字符串**（契约 §3：二进制走 base64 过线，JSON number 数组会放大数倍体积）。
 * **必须分块**：一次性 `String.fromCharCode(...bytes)` 在 0.2–2 MB 的分享图上必然
 * `RangeError: Maximum call stack size exceeded`；分块只拼二进制串，base64 全程只算一次
 * （逐块各自编码会毁掉 3 字节/4 字符的对齐）。空数组 → 空串。
 */
export function base64Of(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  let binary = "";
  for (let i = 0; i < bytes.length; i += BASE64_CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + BASE64_CHUNK));
  }
  return btoa(binary);
}

/**
 * 文本落盘（CSV 用）。`filename` **必须以 `.csv` 结尾**才会拿到 UTF-8 BOM（见文件头约定 ①）。
 * 成功返回落盘绝对路径；失败返回自由文本原因（见约定 ②），**不上抛**——调用方按 `ok` 分派提示。
 */
export async function saveText(
  content: string,
  filename: string,
  deps: SaveDeps = defaultSaveDeps()
): Promise<SaveOutcome> {
  try {
    return { ok: true, path: await deps.exportSaveText(filename, content) };
  } catch (e) {
    return { ok: false, reason: "failed", error: String(e) };
  }
}

/**
 * 二进制落盘（分享图 PNG 用）。`bytes` 经 `base64Of` 变字符串过线（为什么必须分块见该函数）；
 * 空 `bytes` 的坑见文件头约定 ③（当前由固定画布保证不可达：只注明，不改行为）。
 */
export async function saveBytes(
  bytes: Uint8Array,
  filename: string,
  deps: SaveDeps = defaultSaveDeps()
): Promise<SaveOutcome> {
  try {
    return { ok: true, path: await deps.exportSaveBytes(filename, base64Of(bytes)) };
  } catch (e) {
    return { ok: false, reason: "failed", error: String(e) };
  }
}

/**
 * 落盘后在系统文件管理器里**选中该文件**：把**文件路径本身**交给既有 `revealDir`
 * （照抄 ① 的 `UsageStatusSection.tsx` 同一条链路；传父目录只会打开目录，不会选中文件）。
 *
 * **失败原样上抛**（本层不吞、也不翻文案）：① 的口径是「定位失败仍算导出成功」，
 * 提示什么由调用方决定。开发机设了 `MAM_HOME` 时导出目录会重定向、而 reveal 白名单只认
 * `~/.mam` → 这里会真失败，正是这条口径要覆盖的场景。
 */
export async function openContainingDir(
  filePath: string,
  deps: SaveDeps = defaultSaveDeps()
): Promise<void> {
  await deps.revealDir(filePath);
}

/**
 * 文本副本（剪贴板）。**只作副本**：导出的主出口恒为 Rust 落盘，本函数一次 IPC 都不发。
 * 走既有先例 `navigator.clipboard.writeText`；失败（非安全上下文 / 无权限）返回自由文本原因。
 */
export async function copyText(
  text: string,
  deps: SaveDeps = defaultSaveDeps()
): Promise<CopyOutcome> {
  try {
    await deps.clipboardWriteText(text);
    return { ok: true, via: "clipboard" };
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}

/**
 * 图片副本（剪贴板 PNG）。**best-effort 副本**，同样是「不是出口的那条路」：
 * `ClipboardItem` 全仓零先例、secure-context 前提未验证（真机可能根本不可用）→
 * 失败**如实**返回 `{ ok: false, error }`，调用方据此提示「图片已落盘，但没能复制到剪贴板」。
 * **不得**退化成落盘：那是另一条命令的职责，混进来会让「复制」这个动作偷偷写文件。
 */
export async function copyImage(
  blob: Blob,
  deps: SaveDeps = defaultSaveDeps()
): Promise<CopyOutcome> {
  try {
    await deps.clipboardWriteImage(blob);
    return { ok: true, via: "clipboard" };
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}
