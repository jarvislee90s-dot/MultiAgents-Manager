// tests/settings/remoteSection.test.tsx — 评审修复 R2（P8b 收尾）：
// 设置页「本机名称」输入：加载时 get_setting("remote.host_name") 回填当前值，
// blur 时 set_setting 原样落盘（空串语义由后端 display_host_name 过滤，前端不校验）。
// RemoteSection 有既有组件但无既有测试文件，按 toolManagement.test.tsx 的 mock 模式新建。
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, toastInfoMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  toastInfoMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
// useAppTranslation 内部 listen("@tauri-apps/api/event") 在 jsdom 无 Tauri 内核，须 mock
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
// M4 T0b：sonner mock 须放文件顶部——vi.mock 会被提升，写在 it() 内引用局部变量会因 hoisting 报错
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), {
    info: toastInfoMock,
    success: vi.fn(),
    error: vi.fn(),
  }),
}));

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言（jsdom navigator.language=en）
import i18n from "@/i18n";
import { RemoteSection } from "@/components/settings/RemoteSection";

void i18n;

const status = {
  enabled: false,
  bind: "127.0.0.1",
  port: 8787,
  url: "http://127.0.0.1:8787/m",
  lanUrls: [],
  addresses: [{ url: "http://127.0.0.1:8787/m", iface: "", primary: true }],
};

// 2026-09-16 用户裁决：地址合并为**一个区块**逐条展示（多网卡有两个不同网段的
// 地址，旧版「访问地址 + 局域网地址」两块并列被读成重复）
const lanStatus = {
  enabled: true,
  bind: "0.0.0.0",
  port: 9420,
  url: "http://192.168.66.202:9420/m",
  lanUrls: ["http://192.168.66.202:9420/m", "http://192.168.42.216:9420/m"],
  addresses: [
    { url: "http://192.168.66.202:9420/m", iface: "WLAN", primary: true },
    { url: "http://192.168.42.216:9420/m", iface: "以太网", primary: false },
  ],
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
    if (cmd === "remote_status") return { ...status };
    if (cmd === "get_setting") {
      return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
    }
    return null;
  });
});

describe("RemoteSection 访问地址展示（2026-09-16 单区块 + 网卡名）", () => {
  it("多网卡地址在同一个「访问地址」区块逐条展示，各带网卡名与推荐标记", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return { ...lanStatus };
      if (cmd === "get_setting") {
        return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    // 单区块：地址标签只出现一次（旧版是「访问地址」+「局域网地址」两块并列），
    // 且不再有独立的局域网地址分区
    expect(await screen.findAllByText("Address")).toHaveLength(1);
    expect(screen.queryByText("LAN addresses (for the phone)")).toBeNull();
    // 两个地址都在，且逐条标注所属网卡
    expect(screen.getByText("http://192.168.66.202:9420/m")).toBeTruthy();
    expect(screen.getByText("http://192.168.42.216:9420/m")).toBeTruthy();
    expect(screen.getByText("WLAN")).toBeTruthy();
    expect(screen.getByText("以太网")).toBeTruthy();
    // 主（推荐）地址有标记，另一条没有
    expect(screen.getAllByText("Recommended")).toHaveLength(1);
  });

  it("loopback 绑定：单条目（本机），无网卡名与推荐噪声", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") {
        return {
          enabled: true,
          bind: "127.0.0.1",
          port: 9420,
          url: "http://127.0.0.1:9420/m",
          lanUrls: [],
          addresses: [{ url: "http://127.0.0.1:9420/m", iface: "本机", primary: true }],
        };
      }
      if (cmd === "get_setting") {
        return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    expect(await screen.findAllByText("Address")).toHaveLength(1);
    expect(screen.getByText("http://127.0.0.1:9420/m")).toBeTruthy();
    expect(screen.getByText("本机")).toBeTruthy();
  });
});

describe("RemoteSection 本机名称输入（P8b 收尾）", () => {
  it("加载时回填当前值：get_setting(remote.host_name) → 输入框显示已存名称", async () => {
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("JARVIS-Win"));
  });

  it("输入 + blur → set_setting 以 (remote.host_name, 新值) 落盘（原样写，不做非空校验）", async () => {
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("JARVIS-Win"));
    fireEvent.change(input, { target: { value: "My PC" } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_setting", {
        key: "remote.host_name",
        value: "My PC",
      })
    );
  });

  it("加载后未改动即 blur：同样落盘当前值（幂等写，后端 KV 原语义）", async () => {
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("JARVIS-Win"));
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_setting", {
        key: "remote.host_name",
        value: "JARVIS-Win",
      })
    );
  });
});

// M4 T0b 语义保留（M4 Task 4 改写）：TLS 确认「只进不退、不可在线撤回」。常驻勾选框
// 退场后，该语义表现为：已确认（acked）状态下开启不再走任何确认、remote_confirm_public
// 不再被调（KV 只置位一次），且 UI 无撤销/再确认入口（勾选框已删，由下方用例 F 锁定）。
// 原用例的「取消勾选回弹 + toast」分支随勾选框整块退场，toast.info 断言反向保留。
describe("RemoteSection TLS 确认不可在线撤回（M4 T0b，Task 4 改写为开启动作分流）", () => {
  it("tls ack: 已确认状态下点开 → 直调 remote_toggle，不再确认（只进不退）", async () => {
    // bind=0.0.0.0 + 已确认（remote.public_ack = "true"）
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "remote_status") return Promise.resolve({ ...lanStatus, enabled: false });
      if (cmd === "get_setting")
        return Promise.resolve(args?.key === "remote.public_ack" ? "true" : null);
      return Promise.resolve(null);
    });
    render(<RemoteSection />);
    // 「开启远程接入」Switch 在 DOM 中先于「电源保活」Switch（渲染顺序稳定）取第一个；
    // Switch disabled={busy || !status}，先等 status 载入
    const sw = screen.getAllByRole("switch")[0];
    await waitFor(() => expect(sw).toBeEnabled());
    fireEvent.click(sw);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_toggle", { enabled: true })
    );
    // 无 Dialog、无再确认：确认只进不退，撤销唯一路径是改绑本机模式（后端 P7 门不变）
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("remote_confirm_public");
    // 原取消方向 toast 分支已随勾选框删除，不得再触发
    expect(toastInfoMock).not.toHaveBeenCalled();
  });
});

// M4 Task 4（方案 A）：TLS 对外绑定确认改一次性 Dialog——常驻勾选框退场。
// 编排：0.0.0.0 且未确认 → 点开 Switch 先弹 Dialog（不调后端）；确认 =
// remote_confirm_public 置位 remote.public_ack → 再 enable()；取消仅关弹窗；
// 127.0.0.1 与已确认路径直开无感。后端 P7 门 / remote_confirm_public / KV 语义零改动。
describe("RemoteSection TLS 对外绑定确认改一次性 Dialog（M4 Task 4）", () => {
  // 开关定位：enable Switch 无 aria 关联标签，DOM 中先于「电源保活」Switch（渲染顺序
  // 稳定）取第一个；disabled={busy || !status}，点击前先等 status 载入
  const enableSwitch = () => screen.getAllByRole("switch")[0];

  // bind=0.0.0.0（对外）+ enabled=false（待开启）+ public_ack 可选的最小 mock
  const mockLanOff = (ack: string | null) => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return { ...lanStatus, enabled: false };
      if (cmd === "get_setting") return args?.key === "remote.public_ack" ? ack : null;
      return null;
    });
  };

  it("用例 A：bind=0.0.0.0 且未确认 → 点开 Switch 弹 TLS 确认 Dialog，remote_toggle 未被调", async () => {
    mockLanOff(null);
    render(<RemoteSection />);
    await waitFor(() => expect(enableSwitch()).toBeEnabled());
    fireEvent.click(enableSwitch());
    expect(await screen.findByText(/Confirm external binding/i)).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("remote_toggle", expect.anything());
  });

  it("用例 B：Dialog 确认 → remote_confirm_public 先于 remote_toggle(true) 被调，确认后 Dialog 关", async () => {
    const calls: string[] = [];
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      calls.push(cmd);
      if (cmd === "remote_status") return { ...lanStatus, enabled: false };
      if (cmd === "get_setting") return null;
      return null;
    });
    render(<RemoteSection />);
    await waitFor(() => expect(enableSwitch()).toBeEnabled());
    fireEvent.click(enableSwitch());
    fireEvent.click(await screen.findByRole("button", { name: /I understand/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_toggle", { enabled: true })
    );
    // 调用顺序：先置位 ack（后端 P7 门据此放行），再开启——两命令都必须真实发生
    expect(calls).toContain("remote_confirm_public");
    expect(calls.indexOf("remote_confirm_public")).toBeLessThan(
      calls.indexOf("remote_toggle")
    );
    // 确认成功 → Dialog 关
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("用例 C：Dialog 取消 → remote_confirm_public / remote_toggle 均未调，仅关 Dialog", async () => {
    mockLanOff(null);
    render(<RemoteSection />);
    await waitFor(() => expect(enableSwitch()).toBeEnabled());
    fireEvent.click(enableSwitch());
    fireEvent.click(await screen.findByRole("button", { name: /^cancel$/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(invokeMock).not.toHaveBeenCalledWith("remote_confirm_public");
    expect(invokeMock).not.toHaveBeenCalledWith("remote_toggle", expect.anything());
  });

  it("用例 D：bind=127.0.0.1 → 点开直调 remote_toggle(true)，无 Dialog 无确认", async () => {
    render(<RemoteSection />); // 默认 mock：bind=127.0.0.1、enabled=false、public_ack=null
    await waitFor(() => expect(enableSwitch()).toBeEnabled());
    fireEvent.click(enableSwitch());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_toggle", { enabled: true })
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("remote_confirm_public");
  });

  it("用例 E：bind=0.0.0.0 且已确认 → 点开直调 remote_toggle(true)，无 Dialog 不再确认", async () => {
    mockLanOff("true");
    render(<RemoteSection />);
    await waitFor(() => expect(enableSwitch()).toBeEnabled());
    fireEvent.click(enableSwitch());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_toggle", { enabled: true })
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("remote_confirm_public");
  });

  it("用例 F：旧常驻勾选框文案（tlsAck label / 不可撤回说明）不再渲染", async () => {
    mockLanOff(null);
    render(<RemoteSection />);
    await waitFor(() => expect(enableSwitch()).toBeEnabled());
    expect(screen.queryByText(/I confirm a TLS reverse proxy/i)).toBeNull();
    expect(screen.queryByText(/cannot be undone/i)).toBeNull();
  });
});

// M4 T2：待审批面板（4 位码可见 + 批准）与花名册（在线点 + 吊销）
describe("RemoteSection 配对面板与设备花名册（M4 T2）", () => {
  it("pairing panel: pending request shows code, approve works; roster revokes", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "remote_status") return Promise.resolve({ ...lanStatus, enabled: true });
      if (cmd === "get_setting") return Promise.resolve(null);
      if (cmd === "remote_pending_requests")
        return Promise.resolve([
          {
            id: "r0",
            name: "我的手机",
            ip: "192.168.1.9",
            ua: "Mobile Safari",
            code: "2468",
            expiresAt: Date.now() + 300_000,
          },
        ]);
      if (cmd === "remote_devices")
        return Promise.resolve([
          { id: "d1", name: "我的手机", firstPairedAt: 1, lastSeenAt: Date.now(), online: true },
          { id: "d2", name: "", firstPairedAt: 2, lastSeenAt: 1, online: false },
        ]);
      return Promise.resolve(null);
    });
    render(<RemoteSection />);
    // 待审批：设备名 + 来源 IP + 4 位码 + 批准按钮
    expect(await screen.findByText("我的手机")).toBeInTheDocument();
    expect(screen.getByText("2468")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("remote_approve_request", { id: "r0" }));
    // 花名册：在线点 + 单独吊销 + 全部吊销
    expect(await screen.findByRole("button", { name: /revoke all/i })).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: /^revoke$/i })[0]);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("remote_revoke_device", { id: "d1" }));
  });
});

// M4 T1a：通道区块三选一 + named 需 Token + 隧道地址条目徽标
describe("RemoteSection 外部通道区块（M4 T1a）", () => {
  it("channel block: renders selector, saves via remote_set_channel, tunnel badge", async () => {
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "remote_status")
        return Promise.resolve({
          ...lanStatus,
          enabled: true,
          channel: "named",
          tunnelUrl: "https://mam-mac.example.asia",
          tunnelError: null,
          addresses: [
            { url: "https://mam-mac.example.asia", iface: "", primary: true, kind: "tunnel" },
            { url: "http://192.168.66.202:9420/m", iface: "WLAN", primary: false, kind: "lan" },
          ],
        });
      if (cmd === "get_setting") return Promise.resolve(null);
      return Promise.resolve(null);
    });
    render(<RemoteSection />);
    // 先等 status 驱动的内容上屏（通道 label 不依赖 status，首帧即渲染；
    // 直接断言徽标会拿到「只有 label」的首帧 → 假阴），隧道地址条目即 status 已载
    expect(await screen.findByText("https://mam-mac.example.asia")).toBeInTheDocument();
    // 隧道地址条目带「外部通道」徽标且居首；通道区块 label 与徽标是同文案（都是
    // External Channel）→ findAllByText 防多匹配报错（label + 徽标 ≥ 2 处）
    const externalTexts = screen.getAllByText(/external channel/i);
    expect(externalTexts.length).toBeGreaterThanOrEqual(2);
    // URL 同时出现在地址条目与「当前隧道地址」行——用 getAllByText 防多匹配报错
    expect(screen.getAllByText("https://mam-mac.example.asia").length).toBeGreaterThan(0);
    // 通道三按钮：named 高亮
    const namedBtn = screen.getByRole("button", { name: /named tunnel/i });
    expect(namedBtn).toBeInTheDocument();
    // 切到临时隧道 → invoke remote_set_channel
    fireEvent.click(screen.getByRole("button", { name: /quick tunnel/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "remote_set_channel",
        expect.objectContaining({ channel: "quick" })
      )
    );
  });
});

// M4 Task 3：解除命名隧道 Token 死锁。根因：旧代码「选 named 必调后端 → 后端空 Token
// 必拒 → 面板只在 channel===named 才渲染」三者互锁——无 Token 永远切不成 named，
// 输入框永不出现，无处填 Token。新编排：空 Token 点 named 不调后端，直接展开
// Token 面板 + 三步引导 + 文档外链 + 警示；取消按钮仅「待切换」态（channel 非 named）有语义。
describe("RemoteSection 命名隧道 Token 面板（M4 Task 3 死锁解除）", () => {
  const TUNNEL_DOC_URL =
    "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/";

  it("用例 A（死锁解除核心）：channel=off 且 Token 为空 → 点「命名隧道」不调后端，面板与三步引导展开", async () => {
    render(<RemoteSection />);
    // 通道区块不依赖 status 加载，named 按钮（channel 缺省按 off）首帧即可点
    fireEvent.click(screen.getByRole("button", { name: /named tunnel/i }));
    // 核心断言：后端零调用——remote_set_channel 从未被发起（旧代码此处必被调用）
    expect(invokeMock).not.toHaveBeenCalledWith("remote_set_channel", expect.anything());
    // 面板展开：Token 输入框 + 三步引导 + Cloudflare 文档外链 + 警示行
    //（步骤断言用片段避开 tunnelTokenHint——其文案同样含「Zero Trust / Tunnels」）
    expect(await screen.findByLabelText("Tunnel Token")).toBeInTheDocument();
    expect(screen.getByText(/cloudflare dashboard/i)).toBeInTheDocument();
    expect(screen.getByText(/create a tunnel/i)).toBeInTheDocument();
    expect(screen.getByText(/paste it here/i)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /cloudflare docs/i })).toHaveAttribute(
      "href",
      TUNNEL_DOC_URL
    );
    expect(screen.getByText(/never share/i)).toBeInTheDocument();
  });

  it("用例 B：面板展开后填 Token 点保存 → remote_set_channel 以 (named, token) 调用", async () => {
    render(<RemoteSection />);
    fireEvent.click(screen.getByRole("button", { name: /named tunnel/i }));
    const input = (await screen.findByLabelText("Tunnel Token")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "eyJh-test-token" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_set_channel", {
        channel: "named",
        token: "eyJh-test-token",
      })
    );
  });

  it("用例 C：待切换态（channel 非 named）点取消 → 面板消失且不调后端", async () => {
    render(<RemoteSection />);
    fireEvent.click(screen.getByRole("button", { name: /named tunnel/i }));
    expect(await screen.findByLabelText("Tunnel Token")).toBeInTheDocument();
    // 取消按钮只在「待切换」态出现（channel 尚非 named）
    fireEvent.click(screen.getByRole("button", { name: /^cancel$/i }));
    await act(async () => {}); // 冲取消回读 get_setting 的微任务链（评审 Minor 1，防 act 警告）
    expect(screen.queryByLabelText("Tunnel Token")).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith("remote_set_channel", expect.anything());
  });

  it("用例 D（不回归）：channel 已为 named → 输入框照常渲染并回填已存 Token，且无取消按钮", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return { ...status, channel: "named" };
      if (cmd === "get_setting") {
        return args?.key === "remote.tunnel_token" ? "eyJh-saved-token" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Tunnel Token")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("eyJh-saved-token"));
    // 已切 named = 常驻配置面板：无取消按钮（取消仅待切换态）
    expect(screen.queryByRole("button", { name: /^cancel$/i })).toBeNull();
  });

  // 2026-09-17 评审 Important：tokenPanelOpen 残留——待切换面板开着时点 quick/off，
  // 后端切换成功但面板继续挂在非 named 通道下（孤儿 UI）。修后：调后端前先收面板。
  it("用例 E：待切换面板开着 → 点「临时隧道」→ remote_set_channel(quick) 且面板收起", async () => {
    render(<RemoteSection />);
    fireEvent.click(screen.getByRole("button", { name: /named tunnel/i }));
    expect(await screen.findByLabelText("Tunnel Token")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /quick tunnel/i }));
    // 后端切换照常发起（quick 即调即热生效）
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "remote_set_channel",
        expect.objectContaining({ channel: "quick" })
      )
    );
    // 面板随切离 named 收起：Token 输入框从文档消失
    expect(screen.queryByLabelText("Tunnel Token")).toBeNull();
  });

  // 2026-09-17 评审 Minor 2 缺口：挂载回填场景——已存 Token 时点 named 应直调后端
  // 携行落库值（非空直调是既有语义），且不得误入「待切换」面板
  it("用例 F：channel=off 且已存 Token → 点「命名隧道」以 (named, 已存Token) 直调，无待切换面板", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return { ...status };
      if (cmd === "get_setting") {
        return args?.key === "remote.tunnel_token" ? "eyJh-saved-token" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    // 等挂载回填完成（token state 非空后方可点 named，否则会误入待切换面板路径）
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("get_setting", { key: "remote.tunnel_token" })
    );
    await act(async () => {}); // 冲 setToken 微任务续体
    fireEvent.click(screen.getByRole("button", { name: /named tunnel/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_set_channel", {
        channel: "named",
        token: "eyJh-saved-token",
      })
    );
    // 直调路径：待切换面板（含取消按钮）全程不出现
    expect(screen.queryByLabelText("Tunnel Token")).toBeNull();
    expect(screen.queryByRole("button", { name: /^cancel$/i })).toBeNull();
  });

  // 2026-09-17 评审 Minor 1：取消应弃用半截 Token——否则 token state 残留半截值，
  // 再点「命名隧道」会因非空把半截 Token 直调写库（后端只查非空照收，隧道必失败）
  it("用例 G：待切换态输入半截 Token 点取消 → 弃用半截值，再点「命名隧道」不直调写库", async () => {
    render(<RemoteSection />);
    fireEvent.click(screen.getByRole("button", { name: /named tunnel/i }));
    const input = (await screen.findByLabelText("Tunnel Token")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "eyJh-half-typed" } });
    fireEvent.click(screen.getByRole("button", { name: /^cancel$/i }));
    expect(screen.queryByLabelText("Tunnel Token")).toBeNull();
    await act(async () => {}); // 冲取消回读 get_setting 的微任务链（token 恢复已落库值）
    // 再点 named：token 已恢复已落库值（此场景为空）→ 仍走待切换面板，绝不直调后端
    fireEvent.click(screen.getByRole("button", { name: /named tunnel/i }));
    expect(invokeMock).not.toHaveBeenCalledWith("remote_set_channel", expect.anything());
    expect(await screen.findByLabelText("Tunnel Token")).toBeInTheDocument();
  });
});

// M4 Task 2：设置页随 3s 轮询刷新隧道状态——地址常驻不再只靠 toast。
// 根因：remote_status 只在进面板/开关/改配置动作时读一次；隧道拉起是异步的，
// 地址到手时只有 remote-tunnel-address 事件弹 toast，页面状态区永远空。
// fake timer 用法：本 describe 独占 vi.useFakeTimers()，afterEach 还原（防用例间泄漏）；
// fake timer 只拦宏任务不拦 promise 微任务，故用 await act(async () => {}) 冲异步链，
// 不用 waitFor/findBy（其内部 setInterval 会被 fake timer 卡死）。
describe("RemoteSection 隧道状态随 3s 轮询常驻（M4 Task 2）", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  // 冲干净整条 promise 微任务链（refresh/backfill/interval 回调均为微任务续体）
  const flushAsync = async () => {
    await act(async () => {});
  };

  it("用例 A：隧道地址到手后 ≤3s，地址区与「Current tunnel URL」行自动出现", async () => {
    vi.useFakeTimers();
    // 隧道拉起是异步的：首拍 remote_status 无 tunnelUrl（cloudflared 未返回），3s 后到手
    let tunnelUp = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status") {
        return tunnelUp
          ? {
              ...lanStatus,
              tunnelUrl: "https://mam-win.example.asia",
              addresses: [
                { url: "https://mam-win.example.asia", iface: "", primary: true, kind: "tunnel" },
                { url: "http://192.168.66.202:9420/m", iface: "WLAN", primary: false, kind: "lan" },
              ],
            }
          : { ...lanStatus };
      }
      return null;
    });
    render(<RemoteSection />);
    await flushAsync(); // 冲初始 refresh()：status（enabled=true）上屏，3s interval 挂上
    // 初始：无隧道地址，也无「Current tunnel URL」行
    expect(screen.queryByText("https://mam-win.example.asia")).toBeNull();
    expect(screen.queryByText(/current tunnel url/i)).toBeNull();
    // 地址到手：下一拍 remote_status 返回 tunnelUrl + 隧道地址条目
    tunnelUp = true;
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    // ≤3s 自动出现：地址表隧道条目 + 「Current tunnel URL」行（Task 1 归一的看板地址原样展示）
    expect(screen.getByText("https://mam-win.example.asia")).toBeTruthy();
    expect(screen.getByText(/current tunnel url/i)).toBeTruthy();
  });

  it("用例 B（防 clobber）：轮询运行中，用户正在输入的本机名不被轮询重置", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return { ...lanStatus };
      if (cmd === "get_setting") {
        return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    await flushAsync(); // host_name 回填完成
    const input = screen.getByLabelText("Machine name") as HTMLInputElement;
    expect(input.value).toBe("JARVIS-Win");
    // 用户输入到一半，轮询拍点到达
    fireEvent.change(input, { target: { value: "My PC" } });
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    // 轮询只刷 status、不回读 host_name——输入保持（若误复用 refresh() 此处会被 JARVIS-Win 覆盖）
    expect(input.value).toBe("My PC");
  });
});
