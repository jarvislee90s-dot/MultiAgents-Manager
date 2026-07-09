import "@testing-library/jest-dom";
import { vi } from "vitest";
import { server } from "./msw/server";
import { tauriInvokeMock } from "./msw/tauriMocks";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: tauriInvokeMock,
}));

beforeAll(() => server.listen());
afterEach(() => {
  server.resetHandlers();
  vi.clearAllMocks();
});
afterAll(() => server.close());
