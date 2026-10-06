import "@testing-library/jest-dom";
import { vi } from "vitest";
import { server } from "./msw/server";
import { convertFileSrcMock, tauriInvokeMock } from "./msw/tauriMocks";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: tauriInvokeMock,
  convertFileSrc: convertFileSrcMock,
}));

// `@tauri-apps/api/event` 的**全局替身**（2026-10-07，B1 暴露）：
// `use-app-translation` 的 effect 会 `listen("language-changed")`，而 `listen` 在 jsdom 里
// **没有 `window.__TAURI_INTERNALS__`** ⇒ 抛 `Cannot read properties of undefined (reading
// 'transformCallback')`。这条 rejection 是**异步**的，不会让任何用例变红，却会让整个
// `pnpm test` **退出码 1**（vitest 的 "Unhandled Errors" 判定）—— 实测它一次能积累 36 条。
//
// 为什么放在 setup 而不是各个用例文件：`useAppTranslation` 是**跨组件**的通用 hook，
// 只要某个组件用了它、而该用例文件没替身 `event`，就会漏出去。逐个文件补是打地鼠，
// 而这里补一次全覆盖（`core` 的替身本来就在这儿，两者同源）。
// `listen` 返回可 await 的解除函数，与真实签名一致；`emit` 一并给上以免再漏一个。
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  once: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

beforeAll(() => server.listen());
afterEach(() => {
  server.resetHandlers();
  vi.clearAllMocks();
});
afterAll(() => server.close());
