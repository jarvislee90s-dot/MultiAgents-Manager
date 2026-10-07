import { http, HttpResponse } from "msw";

// FilePanel 挂载即拉 GET /m/api/v1/channel（§C5 通道装饰能力）——默认回局域网
// 满速载荷，消掉 FilePanel 全量测试的 unhandled request 噪音；不承载语义断言
// （需要特定通道载荷的测试在各自文件里 stubGlobal fetch 盖过本 handler）。
export const handlers = [
  http.get("/m/api/v1/channel", () =>
    HttpResponse.json({ via: "lan", limited: false, est_mbps_down: 0, est_mbps_up: 0 })
  ),
];
