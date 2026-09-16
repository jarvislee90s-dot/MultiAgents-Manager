# M4 外网全链路（配对管控 + 隧道 + 保活 + 托盘）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 M4 五组能力——T0 前置清账（SSE 即时断连 / TLS 撤回文案）、T1 外部通道（命名隧道主路径 + 临时隧道 + cloudflared 获取）、T2 审批配对 + 设备花名册、T3 电源保活、T4 托盘远程入口；外网真机验收全绿即宪法一期验收达成。

**Architecture:** 全部在既有 `src-tauri/src/remote/` 模块内扩展：新增 `tunnel.rs`（cloudflared 子进程管理）、`approval.rs`（审批队列状态机）、`power.rs`（电源锁）三个文件；`server.rs` 增 SSE 连接注册表与审批服务；设置页 `RemoteSection.tsx` 增通道区块 / 待审批面板 / 花名册；移动端 `PairPage.tsx` 增请求接入流；托盘 `system_tray.rs` 增远程开关。事件通知统一走全局 `APP_HANDLE` emit → 前端监听。

**Tech Stack:** Rust（axum 0.8 / tokio / reqwest rustls / windows 0.57）、React 19 + TypeScript、vitest + @testing-library、i18next（zh/en）。

**Spec:** `docs/superpowers/specs/2026-09-16-m4-external-fullchain-design.md`（v1.1）——本计划的每条需求都从该 spec 出发；执行者同时读 spec 与本计划。

## Global Constraints

- 门禁（每任务过，收尾任务全量过）：`cd src-tauri && cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`cargo test`；根目录 `pnpm check`（prettier/eslint/i18n/tsc/build）与 `pnpm test`。
- i18n 中英双语键必须成对（`pnpm check` 含 check:i18n 会拦截漏键）；测试断言默认英文文案（jsdom navigator.language=en，见 `tests/settings/remoteSection.test.tsx` 头注释先例）。
- 注释与文档一律中文；实现细节红线：不复制聚合逻辑（P8 数据同源）、gate 放行名单用 nest 剥前缀后的**相对路径**、`with` 持锁不可重入。
- 会话扫描预算契约不触碰（本计划不改任何 adapter/monitor 解析器）。
- Rust 测试不得依赖网络与真实 `~/.mam`（用 `DeviceStore::memory()` / 注入闭包 / tempfile 先例）。
- CI 在 cargo test 前跑 `pnpm build:mobile`——新增移动端代码必须保证构建通过。
- 既有行为零回归：M2 七场景 + M3 全部能力（文件面板/书签/横向分屏）不动语义。

---

### Task 1: T0a · SSE 连接注册表（吊销/停止即时断连）

**Files:**
- Modify: `src-tauri/src/remote/server.rs`（SseRegistry + RemoteState 字段 + 测试）
- Modify: `src-tauri/src/remote/api.rs`（events 注册/断连包装）
- Modify: `src-tauri/src/remote/mod.rs`（stop_server 断连；STATE 构造新字段）

**Interfaces:**
- Produces: `server::SseRegistry::register(device:&str)->(u64, oneshot::Receiver<()>)`、`unregister(device,id)`、`disconnect_device(device)->usize`、`disconnect_all()->usize`、`has(device)->bool`；`RemoteState.sse_registry: Arc<SseRegistry>`（Task 7 在线口径、Task 8 吊销接线消费）。

- [ ] **Step 1: 写失败测试（注册表单元 + 吊销断流集成）**

在 `server.rs` 的 `mod tests` 追加（沿用既有 `test_state()` 构造，先给它加字段——本步先写测试，编译失败即红灯）：

```rust
// ---- SseRegistry 单元（M4 T0a）----

#[test]
fn sse_registry_register_then_disconnect_device() {
    let reg = SseRegistry::default();
    let (id1, mut rx1) = reg.register("dev-a");
    let (_id2, rx2) = reg.register("dev-a");
    let (_id3, _rx3) = reg.register("dev-b");
    assert!(reg.has("dev-a") && reg.has("dev-b"));
    // 断 dev-a：两条连接全断，dev-b 不受影响
    assert_eq!(reg.disconnect_device("dev-a"), 2);
    assert!(rx1.try_recv().is_ok());
    drop(rx2); // 第二条 receiver 被 send 唤醒后丢弃即可（Sender 已 send）
    assert!(!reg.has("dev-a") && reg.has("dev-b"));
    // 自然断开清理：unregister 幂等
    reg.unregister("dev-a", id1);
    reg.unregister("dev-a", id1); // 不 panic
    assert_eq!(reg.disconnect_all(), 1); // 只剩 dev-b
}

#[test]
fn sse_registry_sender_dropped_when_entry_replaced_by_disconnect() {
    let reg = SseRegistry::default();
    let (_id, rx) = reg.register("dev");
    reg.disconnect_device("dev");
    // 断连后 Sender 已移除；receiver 侧已收到信号
    assert!(matches!(rx.try_recv(), Ok(_)));
}

// ---- 吊销即时断流集成（SSE 流随 disconnect 终止）----

#[tokio::test]
async fn sse_stream_ends_when_device_disconnected() {
    let state = test_state();
    // 直通配对拿有效 cookie（复用既有 pair 流程的简化版：直接 persist 一个设备）
    state.store.with(|c| {
        let _ = crate::remote::pairing::persist_device(
            c,
            &crate::remote::pairing::NewDevice {
                id: "dev-sse".into(),
                name: String::new(),
                ua: String::new(),
                origin_ip: String::new(),
                paired_at: 0,
            },
        );
    });
    let app = super::router_with_static(state.clone());
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/m/api/v1/events")
                .header("cookie", "mam_device=dev-sse")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let mut stream = resp.into_body().into_data_stream();
    // 读到首帧（snapshot）证明流已建立
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("首帧超时")
        .unwrap();
    assert!(first.is_ok());
    // 吊销 → 流必须结束（next 返回 None）
    assert_eq!(state.sse_registry.disconnect_device("dev-sse"), 1);
    let end = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("断连后流未在 2s 内结束");
    assert!(end.is_none());
}
```

注意：`test_state()` 需补 `sse_registry: Arc::new(SseRegistry::default())`（本任务 Step 3 一并加）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test sse_`
Expected: 编译失败（`SseRegistry` 未定义）——即红灯。

- [ ] **Step 3: 实现 SseRegistry + events 接线**

`server.rs` 新增（放在 `RemoteState` 定义之前）：

```rust
/// SSE 连接注册表（M4 T0a）：device_id → 活跃连接句柄表。
/// 断连语义：吊销/停止时对目标设备的全部连接发 oneshot 关闭信号，
/// SSE 流的 `take_until` 收到信号即终止 → axum 关闭该 HTTP 连接；
/// 自然断开（客户端关页）由 CleanupStream 的 Drop 反注册。
/// 锁粒度：单 Mutex 短临界区（register/unregister/disconnect 均无 IO），
/// 不与 store/pairing 锁嵌套（锁序红线：registry 永远最后进最先出）。
#[derive(Default)]
pub struct SseRegistry {
    inner: std::sync::Mutex<std::collections::HashMap<String, Vec<(u64, tokio::sync::oneshot::Sender<()>)>>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl SseRegistry {
    /// 注册一条连接：返回 (连接 id, 关闭信号接收端)。
    /// 返回的 Receiver 在 disconnect_device/disconnect_all 或 Sender 被 drop 时给出信号
    pub fn register(&self, device: &str) -> (u64, tokio::sync::oneshot::Receiver<()>) {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inner
            .lock()
            .unwrap()
            .entry(device.to_string())
            .or_default()
            .push((id, tx));
        (id, rx)
    }

    /// 自然断开反注册（幂等：未知 id 静默忽略）
    pub fn unregister(&self, device: &str, id: u64) {
        self.inner.lock().unwrap().get_mut(device).map(|v| {
            v.retain(|(i, _)| *i != id);
        });
    }

    /// 断开指定设备的全部连接，返回断开数
    pub fn disconnect_device(&self, device: &str) -> usize {
        self.inner
            .lock()
            .unwrap()
            .remove(device)
            .map(|v| {
                for (_, tx) in &v {
                    let _ = tx.send(());
                }
                v.len()
            })
            .unwrap_or(0)
    }

    /// 断开全部设备连接（停止远程 / 全部吊销），返回断开数
    pub fn disconnect_all(&self) -> usize {
        let mut map = self.inner.lock().unwrap();
        let n: usize = map.values().map(Vec::len).sum();
        for v in map.values() {
            for (_, tx) in v {
                let _ = tx.send(());
            }
        }
        map.clear();
        n
    }

    /// 该设备是否存在活跃连接（Task 7 花名册在线口径数据源）
    pub fn has(&self, device: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .get(device)
            .is_some_and(|v| !v.is_empty())
    }
}

/// 自然断开清理包装：axum drop SSE 流时反注册注册表项（不留陈旧句柄泄漏）
struct CleanupStream<S> {
    inner: S,
    reg: std::sync::Arc<SseRegistry>,
    device: String,
    conn_id: u64,
}
impl<S: futures::Stream> futures::Stream for CleanupStream<S> {
    type Item = S::Item;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // Safety: 无 Unpin 约束需求经字段投影转发（inner 已被 take_until 包装为 Unpin 流链）
        unsafe { self.map_unchecked_mut(|s| &mut s.inner).poll_next(cx) }
    }
}
impl<S> Drop for CleanupStream<S> {
    fn drop(&mut self) {
        self.reg.unregister(&self.device, self.conn_id);
    }
}
```

`RemoteState` 加字段：

```rust
pub struct RemoteState {
    // ...既有字段不动...
    /// SSE 连接注册表（M4 T0a）：吊销/停止即时断连 + 在线口径数据源
    pub sse_registry: std::sync::Arc<SseRegistry>,
}
```

`api.rs` 的 `events`：函数签名加 `headers: axum::http::HeaderMap`（axum 自动注入），在订阅 `rx` 之后、快照之前插注册逻辑，末尾换返回流：

```rust
pub async fn events(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let rx = st.watcher_tx.subscribe();
    // M4 T0a：注册连接（gate 已过闸，此处必能取到设备 id；防御性 None 时跳过注册只服务）
    let device = super::gate::extract_device(&headers).unwrap_or_default();
    let (conn_id, close_rx) = st.sse_registry.register(&device);
    // ...既有快照/initial/transitions 构造不动...
    let base = initial.chain(transitions);
    // 吊销/停止信号到达即终止流（take_until：close_rx 被 send 或 Sender drop 均触发）
    let guarded = base.take_until(close_rx);
    let cleaned = CleanupStream {
        inner: guarded,
        reg: st.sse_registry.clone(),
        device,
        conn_id,
    };
    Sse::new(cleaned).keep_alive(sse::KeepAlive::default())
}
```

（`take_until` 来自已导入的 `futures::StreamExt as _`；`Sse::new(cleaned)` 需 `CleanupStream: Stream`——已实现。）

`mod.rs` 两处：
1. `STATE` 构造加 `sse_registry: std::sync::Arc::new(server::SseRegistry::default()),`
2. `stop_server()` 在 `h.abort()` 之后加：

```rust
    // M4 T0a：停止 = 已建立 SSE 连接即时断开（旧限制「仅拒新连」的修复前半；
    // 单设备吊销断连在 Task 8 的 remote_revoke_device 接线）
    STATE.sse_registry.disconnect_all();
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test sse_ && cargo test remote`
Expected: 新增 3 测试全过，既有 remote 测试零回归。

- [ ] **Step 5: 门禁 + 提交**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo fmt`
```bash
git add src-tauri/src/remote/server.rs src-tauri/src/remote/api.rs src-tauri/src/remote/mod.rs
git commit -m "feat(m4-t0a): SSE 连接注册表——吊销/停止即时断连（take_until+Drop 反注册），修复 M3 已知限制 1/4 前半"
```

---

### Task 2: T0b · TLS 确认撤回文案（M2 终审留办）

**Files:**
- Modify: `src/components/settings/RemoteSection.tsx:242-249`（Checkbox else 分支 + 常驻说明）
- Modify: `src/i18n/locales/zh.json`、`src/i18n/locales/en.json`（settings.remote 下 3 个新键）
- Test: `tests/settings/remoteSection.test.tsx`（追加用例）

**Interfaces:**
- Consumes: 既有 `acked` state / `toast`（sonner）/ i18n。
- Produces: i18n 键 `settings.remote.tlsAckHint`（常驻说明）、`tlsAckNoRevoke`（取消时 toast）、`tlsAckRevokeByRebind`（toast 第二句）。

- [ ] **Step 1: 写失败测试**

`tests/settings/remoteSection.test.tsx` 追加（沿用文件头既有 `invokeMock` / status 夹具模式；`sonner` 的 toast 需 mock——文件若未 mock 则在顶部补）：

```tsx
// M4 T0b：TLS 确认不可在线撤回——取消勾选回弹 + toast 说明，复选框旁常驻文案
it("tls ack: uncheck snaps back with toast, stays checked", async () => {
  const toastInfo = vi.fn();
  vi.mock("sonner", () => ({
    toast: Object.assign(vi.fn(), {
      info: toastInfo,
      success: vi.fn(),
      error: vi.fn(),
    }),
  }));
  // bind=0.0.0.0 + acked=true 才渲染复选框
  invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "remote_status") return Promise.resolve({ ...lanStatus, enabled: true });
    if (cmd === "get_setting")
      return Promise.resolve(args?.key === "remote.public_ack" ? "true" : null);
    return Promise.resolve(null);
  });
  render(<RemoteSection />);
  const cb = await screen.findByRole("checkbox");
  expect(cb).toBeChecked();
  // 常驻说明文案存在
  expect(screen.getByText(/cannot be undone/i)).toBeInTheDocument();
  // 取消勾选：仍选中 + toast 提示
  fireEvent.click(cb);
  expect(cb).toBeChecked(); // 回弹（受控于 acked，本就 snaps back；本任务补 toast 与文案）
  await waitFor(() => expect(toastInfo).toHaveBeenCalled());
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run tests/settings/remoteSection.test.tsx`
Expected: FAIL——`cannot be undone` 文案不存在。

- [ ] **Step 3: 实现**

`zh.json` / `en.json` 的 `settings.remote` 追加（两文件键序一致）：

```json
"tlsAckHint": "确认一次性生效，不可在线撤回；如需撤销请改绑本机模式后重新启用",
"tlsAckNoRevoke": "安全确认不可在线撤回",
"tlsAckRevokeByRebind": "如需撤销，请将绑定改回「仅本机」后重新启用",
```

```json
"tlsAckHint": "This confirmation is permanent and cannot be undone online",
"tlsAckNoRevoke": "Security confirmation cannot be undone online",
"tlsAckRevokeByRebind": "To undo it, switch binding back to localhost and re-enable",
```

`RemoteSection.tsx` Checkbox 区块改为：

```tsx
{bind === BIND_LAN && (
  <>
    <div className="flex items-center justify-between py-2.5">
      <label className="text-sm font-medium">{t("settings.remote.tlsAck")}</label>
      <Checkbox
        checked={acked}
        onCheckedChange={(v) => {
          // M4 T0b：确认只进不退——取消方向回弹（受控于 acked 态天然回弹）+ 明示不可撤回
          if (v === true) void confirmPublic();
          else
            toast.info(`${t("settings.remote.tlsAckNoRevoke")}。${t("settings.remote.tlsAckRevokeByRebind")}`);
        }}
      />
    </div>
    <p className="text-muted-foreground pb-2 text-xs">{t("settings.remote.tlsAckHint")}</p>
    <div className="border-t" />
  </>
)}
```

- [ ] **Step 4: 跑测试 + 门禁**

Run: `pnpm vitest run tests/settings/remoteSection.test.tsx && pnpm check`
Expected: 全过（既有用例零回归，i18n 键成对）。

- [ ] **Step 5: 提交**

```bash
git add src/components/settings/RemoteSection.tsx src/i18n/locales/zh.json src/i18n/locales/en.json tests/settings/remoteSection.test.tsx
git commit -m "feat(m4-t0b): TLS 确认撤回文案——取消勾选回弹+toast+常驻说明（M2 终审留办清账）"
```

---

### Task 3: 全局 AppHandle + 事件出口（基建，T1/T2/T4 依赖）

**Files:**
- Modify: `src-tauri/src/lib.rs`（APP_HANDLE OnceLock + setup 装配）
- Create: `src-tauri/src/remote/events.rs`（emit_ui 薄壳 + 审计日志 helper）
- Modify: `src-tauri/src/remote/mod.rs`（`pub mod events;`）

**Interfaces:**
- Produces: `events::emit_ui(event: &str, payload: impl serde::Serialize + Clone)`（无句柄时静默——单测/无窗环境安全）；`events::audit(action: &str, detail: &str)`（`log::info!(target: "remote_audit", ...)`，T2e 审计留痕唯一出口）；`lib.rs::APP_HANDLE: OnceLock<tauri::AppHandle>`。
- Consumes（后续）：Task 5 隧道地址通知、Task 7 配对请求通知、Task 11 托盘。

- [ ] **Step 1: 写失败测试**

`events.rs` 直接实现+测试一体（薄壳函数，红绿同文件）：

```rust
// 后端 → 前端事件出口（M4 基建）：远程模块运行在 tauri::async_runtime 任务里，
// 无窗口上下文——统一经全局 APP_HANDLE emit；无句柄（单测）时静默。
// 审计留痕（T2e）同文件收口：全部走 remote_audit target，日志侧可追溯。

use once_cell::sync::Lazy;
use tauri::Emitter;

/// lib.rs setup 里 set 的全局句柄（远程模块不直接依赖 tauri runtime 上下文）
pub static APP_HANDLE: Lazy<Option<tauri::AppHandle>> = Lazy::new(None);

/// 向前端 emit 事件；无句柄或 emit 失败静默降级（通知是尽力而为，不阻断主流程）
pub fn emit_ui(event: &str, payload: impl serde::Serialize + Clone) {
    if let Some(app) = APP_HANDLE.as_ref() {
        if let Err(e) = app.emit(event, payload) {
            log::warn!("emit {event} 失败: {e}");
        }
    }
}

/// 配对/吊销/停止动作审计留痕（T2e）：单行结构化，日志可 grep `remote_audit`
pub fn audit(action: &str, detail: &str) {
    log::info!(target: "remote_audit", "{action} {detail}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_ui_without_handle_is_silent() {
        // 无句柄不 panic（单测环境 Lazy 为 None）
        emit_ui("remote-pair-request", serde_json::json!({"name": "x"}));
    }

    #[test]
    fn audit_writes_log_line() {
        audit("pair_request", "id=r1 ip=127.0.0.1");
    }
}
```

- [ ] **Step 2: 接线（等效"跑通"）**

`lib.rs`：`setup` 闭包内、`crate::remote::restore_on_launch();` **之前**加：

```rust
            // M4：全局句柄落位（远程模块 emit 通知的唯一通道；OnceLock 先到先得）
            let _ = crate::remote::events::APP_HANDLE_SET.call_once({
                let handle = app.handle().clone();
                move || {
                    // 直接写 Lazy 内部 Option 不可行——改用下方 OnceLock 方案
                    unreachable!()
                }
            });
```

**修正**（上面占位不可用——`Lazy<Option<..>>` 无 setter）。`events.rs` 改用 `OnceLock`：

```rust
use std::sync::OnceLock;
pub static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
pub fn emit_ui(event: &str, payload: impl serde::Serialize + Clone) {
    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit(event, payload) {
            log::warn!("emit {event} 失败: {e}");
        }
    }
}
```

`lib.rs` setup 内实际写入：

```rust
            // M4：全局句柄落位（先于 restore_on_launch——隧道自启即可发通知）
            let _ = crate::remote::events::APP_HANDLE.set(app.handle().clone());
            crate::remote::restore_on_launch();
```

`mod.rs` 加 `pub mod events;`。

- [ ] **Step 3: 跑测试 + 门禁**

Run: `cd src-tauri && cargo test events && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 2 测试过，clippy 零告警。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/remote/events.rs src-tauri/src/remote/mod.rs src-tauri/src/lib.rs
git commit -m "feat(m4): 全局 AppHandle + emit_ui/audit 事件出口基建（隧道/配对/托盘通知共用）"
```

---

### Task 4: T1d · cloudflared 获取（发现/下载/解压）

**Files:**
- Create: `src-tauri/src/remote/tunnel.rs`
- Modify: `src-tauri/src/remote/mod.rs`（`pub mod tunnel;` + 两个 KEY 常量）

**Interfaces:**
- Produces: `tunnel::KEY_CHANNEL_VALUE_OFF/QUICK/NAMED`（"off"/"quick"/"named"）；`tunnel::parse_channel(Option<&str>) -> Option<&'static str>`（纯函数）；`tunnel::download_url_for() -> Result<String, String>`（按平台/架构纯函数）；`tunnel::cloudflared_path() -> std::path::PathBuf`（`~/.mam/bin/cloudflared[.exe]`）；`tunnel::ensure_cloudflared(dl: impl Fn(&str, &Path) -> Result<(), String>) -> Result<PathBuf, String>`（注入下载器，本体只管存在性判定+解压落位）。
- Consumes: `mod.rs` 新增 `pub const KEY_CHANNEL: &str = "remote.channel";`、`pub const KEY_TUNNEL_TOKEN: &str = "remote.tunnel_token";`。

- [ ] **Step 1: 写失败测试**

`tunnel.rs` 尾部 `mod tests`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_parse_only_three_literals() {
        assert_eq!(parse_channel(None), Some("off"));
        assert_eq!(parse_channel(Some("")), Some("off"));
        assert_eq!(parse_channel(Some("quick")), Some("quick"));
        assert_eq!(parse_channel(Some("named")), Some("named"));
        assert_eq!(parse_channel(Some("QUICK")), None); // 大小写敏感拒绝
        assert_eq!(parse_channel(Some("tls")), None);
    }

    #[test]
    fn download_url_matches_platform() {
        // 纯函数按 cfg 分支断言本机平台（跨平台 CI 各自命中各自分支）
        let url = download_url_for().unwrap();
        if cfg!(target_os = "macos") {
            assert!(url.contains("cloudflared-darwin-"));
            assert!(url.ends_with(".tgz"));
        } else if cfg!(windows) {
            assert!(url.ends_with("cloudflared-windows-amd64.exe"));
        }
        assert!(url.starts_with("https://github.com/cloudflare/cloudflared/releases/"));
    }

    #[test]
    fn ensure_skips_download_when_binary_exists() {
        // 已存在（含可执行位）→ 不调下载器直接 Ok
        let dir = tempfile::tempdir().unwrap();
        // cloudflared_path 读 HOME——本测试改为直接驱动 ensure_cloudflared 的
        // exists 判定内核：ensure_with(bin_path, downloader)
        let bin = dir.path().join(bin_name());
        #[cfg(unix)]
        std::fs::write(&bin, b"fake").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        std::fs::write(&bin, b"fake").unwrap();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c2 = called.clone();
        let r = ensure_with(
            &bin,
            Box::new(move |_url, _dest| {
                c2.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }),
        );
        assert!(r.is_ok());
        assert!(!called.load(std::sync::atomic::Ordering::Relaxed));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test tunnel`
Expected: 编译失败（模块为空）。

- [ ] **Step 3: 实现**

`tunnel.rs` 主体（进程管理在 Task 5 追加，本文件先立获取层）：

```rust
// 外部通道（M4 T1）：cloudflared 获取层 + 隧道进程管理（Task 5）。
// 设计出处：docs/superpowers/specs/2026-09-16-m4-external-fullchain-design.md T1b/T1c/T1d

use std::path::{Path, PathBuf};

/// 通道三值（remote.channel 的值域；parse_channel 是唯一解析口）
pub const KEY_CHANNEL_VALUE_OFF: &str = "off";
pub const KEY_CHANNEL_VALUE_QUICK: &str = "quick";
pub const KEY_CHANNEL_VALUE_NAMED: &str = "named";

/// 通道设置解析（纯函数）：None/空串 → off（未配置视为关闭，老用户升级兼容）；
/// 仅认三个字面量，其余 None（写入侧 UI 限三选一，这里防御乱串）
pub fn parse_channel(v: Option<&str>) -> Option<&'static str> {
    match v.unwrap_or("") {
        "" | KEY_CHANNEL_VALUE_OFF => Some(KEY_CHANNEL_VALUE_OFF),
        KEY_CHANNEL_VALUE_QUICK => Some(KEY_CHANNEL_VALUE_QUICK),
        KEY_CHANNEL_VALUE_NAMED => Some(KEY_CHANNEL_VALUE_NAMED),
        _ => None,
    }
}

/// 平台二进制名（Windows 带 .exe 后缀）
pub fn bin_name() -> &'static str {
    if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" }
}

/// cloudflared 固定放置路径（spec T1d：~/.mam/bin/，手动放置旁路即放这里）
pub fn cloudflared_path() -> PathBuf {
    crate::database::mam_data_dir() // 若无此函数则用 dirs::home_dir().join(".mam")
        .unwrap_or_else(|| PathBuf::from(".mam"))
        .join("bin")
        .join(bin_name())
}
```

**注**：先 `grep -n "fn mam_data_dir\|\.mam" src-tauri/src/database/mod.rs` 确认数据目录函数名；若为 `dirs::home_dir()` 散用则按现状写 `dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".mam")`。

```rust
/// 下载源（纯函数）：官方 GitHub Releases latest 固定资产名（2026-09-16 spec T1d 自决默认）
pub fn download_url_for() -> Result<String, String> {
    let asset = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "cloudflared-darwin-arm64.tgz" }
        else { "cloudflared-darwin-amd64.tgz" }
    } else if cfg!(windows) {
        "cloudflared-windows-amd64.exe"
    } else if cfg!(target_arch = "aarch64") {
        "cloudflared-linux-arm64"
    } else {
        "cloudflared-linux-amd64"
    };
    Ok(format!("https://github.com/cloudflare/cloudflared/releases/latest/download/{asset}"))
}

/// 二进制就绪判定（macOS 需可执行位；Windows 只查存在）
fn binary_ready(p: &Path) -> bool {
    if !p.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    true
}

/// 获取内核（可测）：已就绪 → 直接返回；否则调注入下载器（生产实现见 download_to），
/// 下载完成后 macOS 从 .tgz 解压（系统 tar）、赋可执行位；Windows 直接落位
pub fn ensure_with(
    bin: &Path,
    dl: Box<dyn FnOnce(&str, &Path) -> Result<(), String>>,
) -> Result<PathBuf, String> {
    if binary_ready(bin) {
        return Ok(bin.to_path_buf());
    }
    let url = download_url_for()?;
    let tmp = bin.with_extension("download");
    dl(&url, &tmp)?;
    if cfg!(target_os = "macos") {
        // .tgz 内含裸二进制 cloudflared：解到同目录再改名（tar 存在于 macOS/全部 CI）
        let dir = bin.parent().ok_or("cloudflared 路径无父目录")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("建 bin 目录失败: {e}"))?;
        let st = std::process::Command::new("tar")
            .args(["-xzf", tmp.to_str().unwrap_or_default(), "-C", dir.to_str().unwrap_or_default()])
            .status().map_err(|e| format!("tar 启动失败: {e}"))?;
        if !st.success() { return Err("cloudflared 解压失败".into()); }
        let _ = std::fs::remove_file(&tmp);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("赋执行位失败: {e}"))?;
    } else {
        std::fs::rename(&tmp, bin).map_err(|e| format!("落位失败: {e}"))?;
    }
    if binary_ready(bin) { Ok(bin.to_path_buf()) } else { Err("cloudflared 落位后校验失败".into()) }
}

/// 生产下载器（reqwest rustls，进度不回调——设置页显示「下载中」文案即可）
pub async fn download_to(url: &str, dest: &Path) -> Result<(), String> {
    let bytes = reqwest::get(url).await.map_err(|e| format!("下载失败: {e}"))?
        .error_for_status().map_err(|e| format!("下载失败: {e}"))?
        .bytes().await.map_err(|e| format!("下载中断: {e}"))?;
    if let Some(dir) = dest.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败: {e}"))?; }
    std::fs::write(dest, &bytes).map_err(|e| format!("写文件失败: {e}"))
}
```

`mod.rs`：`pub mod tunnel;`（位置在 `pub mod server;` 之后字母序插入 `events`/`tunnel`）+ 常量区追加：

```rust
/// 外部通道（M4 T1）：off / quick / named 三值（tunnel::parse_channel 唯一解析口）
pub const KEY_CHANNEL: &str = "remote.channel";
/// 命名隧道 Tunnel Token（M4 T1b；明文本地存储与设备表同库）
pub const KEY_TUNNEL_TOKEN: &str = "remote.tunnel_token";
```

- [ ] **Step 4: 跑测试确认通过 + 门禁**

Run: `cd src-tauri && cargo test tunnel && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 3 测试过。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/remote/tunnel.rs src-tauri/src/remote/mod.rs
git commit -m "feat(m4-t1d): cloudflared 获取层——三值通道解析/按平台下载源/ensure 注入式内核+tar 解压"
```

---

### Task 5: T1b/T1c · 隧道进程管理 + 状态接线

**Files:**
- Modify: `src-tauri/src/remote/tunnel.rs`（进程管理 + 快照 + 守护）
- Modify: `src-tauri/src/remote/mod.rs`（启停接线 + remote_status/issue_token 扩展 + remote_set_channel 命令）

**Interfaces:**
- Consumes: Task 3 `events::emit_ui`、Task 4 获取层。
- Produces: `tunnel::TunnelStatus { mode, url: Option<String>, error: Option<String> }`（`tunnel::snapshot() -> TunnelStatus` 读全局快照）；`tunnel::start_if_configured(port: u16)` / `tunnel::stop()`；`tunnel::restart_if_running(port)`；纯函数 `parse_quick_url(&str) -> Option<String>`、`backoff_ms(u32) -> Option<u64>`、`tunnel_address_entry(url:&str) -> serde_json::Value`。IPC：`remote_set_channel(channel: String, token: Option<String>) -> Result<(), String>`。status 新键：`channel`/`tunnelUrl`/`tunnelError`；`addresses` 条目新增 `kind` 字段（"tunnel"|"lan"，隧道条目恒首位 primary）。

- [ ] **Step 1: 写失败测试**

`tunnel.rs` tests 追加：

```rust
    #[test]
    fn quick_url_parses_only_trycloudflare_host() {
        let line = r#"2026-09-16T00:00:00Z INF |  https://example-words-here.trycloudflare.com"#;
        assert_eq!(
            parse_quick_url(line).as_deref(),
            Some("https://example-words-here.trycloudflare.com")
        );
        // 普通日志行/其他域名不误报
        assert_eq!(parse_quick_url("INF Registered tunnel connection"), None);
        assert_eq!(parse_quick_url("see https://docs.cloudflare.com/"), None);
    }

    #[test]
    fn backoff_is_1s_2s_4s_then_give_up() {
        assert_eq!(backoff_ms(1), Some(1000));
        assert_eq!(backoff_ms(2), Some(2000));
        assert_eq!(backoff_ms(3), Some(4000));
        assert_eq!(backoff_ms(4), None); // 连续失败 3 次后停止（spec T1b）
    }

    #[test]
    fn tunnel_address_entry_is_primary_tunnel_kind() {
        let e = tunnel_address_entry("https://mam.example.asia");
        assert_eq!(e["url"], serde_json::json!("https://mam.example.asia"));
        assert_eq!(e["kind"], serde_json::json!("tunnel"));
        assert_eq!(e["primary"], serde_json::json!(true));
        assert_eq!(e["iface"], serde_json::json!("")); // 前端按 kind 渲染「外部通道」徽标
    }

    #[test]
    fn address_entries_prepend_tunnel() {
        // mod.rs 的 address_entries_with_tunnel：隧道地址恒插到表首位
        let base = vec![serde_json::json!({"url": "http://192.168.1.5:9420/m", "iface": "en0", "primary": true, "kind": "lan"})];
        let out = super::super::address_entries_with_tunnel(
            Some("https://mam.example.asia"),
            base.clone(),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["kind"], serde_json::json!("tunnel"));
        assert_eq!(out[1]["kind"], serde_json::json!("lan"));
        // 无隧道 → 原样（既有条目补 kind="lan"）
        let out2 = super::super::address_entries_with_tunnel(None, base);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0]["kind"], serde_json::json!("lan"));
    }
```

`mod.rs` tests 追加（issue_token 隧道跟随的纯核）：

```rust
    #[test]
    fn pair_url_prefers_tunnel_base() {
        // 隧道开 → https://host/m#token=…；隧道关 → 既有 http 逻辑
        assert_eq!(
            pair_url_with_tunnel(Some("https://mam.example.asia".into()), "http://192.168.1.5:9420/m".into(), "tok".into()),
            "https://mam.example.asia/m#token=tok"
        );
        assert_eq!(
            pair_url_with_tunnel(None, "http://192.168.1.5:9420/m".into(), "tok".into()),
            "http://192.168.1.5:9420/m#token=tok"
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test tunnel && cargo test pair_url`
Expected: 编译失败。

- [ ] **Step 3: 实现**

`tunnel.rs` 追加进程管理层：

```rust
// ============================================================
// 隧道进程管理（T1b/T1c）：spawn → stderr 解析(quick) → 守护重启 → 快照
// ============================================================

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// 对外快照（remote_status 消费；ui 线程与 supervisor 任务共写——Mutex 单字段写短临界区）
#[derive(Clone, Default, serde::Serialize)]
pub struct TunnelStatus {
    pub mode: String,                    // off/quick/named（当前实际运行模式）
    pub url: Option<String>,            // quick=trycloudflare 地址 / named=子域（未知时 None）
    pub error: Option<String>,          // 起不来/连续失败的终态错误（界面明示）
}
static SNAPSHOT: Lazy<Mutex<TunnelStatus>> = Lazy::new(|| {
    Mutex::new(TunnelStatus { mode: KEY_CHANNEL_VALUE_OFF.into(), url: None, error: None })
});

pub fn snapshot() -> TunnelStatus { SNAPSHOT.lock().unwrap().clone() }
fn set_snapshot(f: impl FnOnce(&mut TunnelStatus)) { f(&mut SNAPSHOT.lock().unwrap()); }

/// 运行句柄：stop 信号 + 子进程共享引用 + supervisor 任务句柄
struct TunnelHandle {
    stop: Arc<AtomicBool>,
    child: Arc<tokio::sync::Mutex<tokio::process::Child>>,
    supervisor: tauri::async_runtime::JoinHandle<()>,
}
static TUNNEL: Lazy<Mutex<Option<TunnelHandle>>> = Lazy::new(|| Mutex::new(None));

/// quick stderr 地址解析（纯函数）：行内 https://*.trycloudflare.com 才算
pub fn parse_quick_url(line: &str) -> Option<String> {
    let (s, e) = (line.find("https://")?, line.find(".trycloudflare.com")?);
    if e <= s { return None; }
    Some(line[s..].split_whitespace().next()?.to_string())
}

/// 守护退避（纯函数）：连续失败 n 次后的下次重启等待；None = 放弃
pub fn backoff_ms(consecutive_failures: u32) -> Option<u64> {
    match consecutive_failures {
        1 => Some(1_000),
        2 => Some(2_000),
        3 => Some(4_000),
        _ => None,
    }
}

/// 按当前设置启动隧道（幂等：运行中直接返回；off 直接返回）。
/// 失败不返回 Err（隧道失败不阻断远程主体——spec T1d「不阻塞其他功能」），
/// 错误写快照 error 供设置页展示
pub fn start_if_configured(port: u16) {
    let Some(mode) = parse_channel(crate::database::dao::settings::get_setting(
        crate::remote::KEY_CHANNEL,
    ).as_deref()) else {
        set_snapshot(|s| s.error = Some("通道设置非法".into()));
        return;
    };
    if mode == KEY_CHANNEL_VALUE_OFF {
        return; // 关闭态：快照保持 off
    }
    if mode == KEY_CHANNEL_VALUE_NAMED
        && crate::database::dao::settings::get_setting(crate::remote::KEY_TUNNEL_TOKEN)
            .map(|t| t.trim().is_empty())
            .unwrap_or(true)
    {
        set_snapshot(|s| { s.mode = mode.into(); s.error = Some("命名隧道缺少 Tunnel Token".into()); });
        return;
    }
    let mut h = TUNNEL.lock().unwrap();
    if h.is_some() {
        return; // 运行中幂等（通道切换走 restart 路径）
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let supervisor = tauri::async_runtime::spawn(async move {
        supervise(mode.to_string(), port, stop2).await;
    });
    *h = Some(TunnelHandle { stop, child: Arc::new(tokio::sync::Mutex::new(dummy_child())), supervisor });
    // 注：child 占位由 supervise 内部重建——见 supervise 注释
}

fn dummy_child() -> tokio::process::Child {
    unreachable!("占位不可达：TunnelHandle.child 在 supervise 首圈被真实子进程替换")
}
```

**实现修正**（占位不可取——改为 supervisor 全权持有子进程，`TunnelHandle` 只留 stop + supervisor；kill 经由 stop 标志 + supervisor 内 kill）：

```rust
struct TunnelHandle {
    stop: Arc<AtomicBool>,
    supervisor: tauri::async_runtime::JoinHandle<()>,
}

/// 守护主循环：确保二进制 → spawn → 监听 stderr(quick 解析地址) → wait → 退避重启
async fn supervise(mode: String, port: u16, stop: Arc<AtomicBool>) {
    set_snapshot(|s| { s.mode = mode.clone(); s.url = None; s.error = None; });
    // 获取二进制（可能触发下载——秒到分钟级，全程不阻塞调用方）
    let dl = |url: &str, dest: &Path| {
        let (u, d) = (url.to_string(), dest.to_path_buf());
        tauri::async_runtime::block_on(download_to(&u, &d))
    };
    let bin = match ensure_with(&cloudflared_path(), Box::new(dl)) {
        Ok(p) => p,
        Err(e) => {
            set_snapshot(|s| s.error = Some(format!("cloudflared 获取失败: {e}；可手动放置到 ~/.mam/bin/{}", bin_name())));
            return;
        }
    };
    let mut failures: u32 = 0;
    let mut last_url: Option<String> = None;
    loop {
        if stop.load(Ordering::Relaxed) { break; }
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.arg("tunnel").arg("--no-autoupdate");
        if mode == KEY_CHANNEL_VALUE_QUICK {
            cmd.arg("--url").arg(format!("http://127.0.0.1:{port}"));
        } else {
            let token = crate::database::dao::settings::get_setting(crate::remote::KEY_TUNNEL_TOKEN)
                .unwrap_or_default();
            cmd.arg("run").arg("--token").arg(token);
        }
        cmd.stderr(std::process::Stdio::piped()).stdout(std::process::Stdio::null());
        let mut child = match cmd.spawn() { Ok(c) => c, Err(e) => {
            set_snapshot(|s| s.error = Some(format!("cloudflared 启动失败: {e}")));
            return;
        }};
        // stderr 读取任务（quick 地址解析；named 也无害——named stderr 无 trycloudflare 域名）
        let stderr = child.stderr.take();
        let stop_r = stop.clone();
        let url_sink = Arc::new(Mutex::new(Option::<String>::None));
        let url_sink2 = url_sink.clone();
        if let Some(err) = stderr {
            tauri::async_runtime::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(err).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            if let Some(u) = parse_quick_url(&line) {
                                *url_sink2.lock().unwrap() = Some(u);
                            }
                        }
                        _ => break,
                    }
                }
            });
        }
        // 等子进程退出（或被 stop 方 kill）
        let status = child.wait().await;
        if stop.load(Ordering::Relaxed) { break; } // 主动停止不算失败
        // quick：地址可能已解析——先取走
        let new_url = url_sink.lock().unwrap().take();
        let _ = &mut last_url;
        failures += 1;
        match backoff_ms(failures) {
            Some(ms) => {
                log::warn!("cloudflared 退出({status:?})，{ms}ms 后第 {failures} 次重启");
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
            None => {
                set_snapshot(|s| { s.error = Some("cloudflared 连续失败 3 次，已停止守护".into()); });
                crate::remote::events::emit_ui("remote-tunnel-error", snapshot());
                return;
            }
        }
    }
    let _ = last_url;
}
```

**quick 地址/变更通知**：quick 模式下解析到新地址即写快照 + emit（把 stderr 任务里 url 落位处扩为）：

```rust
                            if let Some(u) = parse_quick_url(&line) {
                                let fresh = { let mut g = url_sink2.lock().unwrap();
                                    let fresh = g.as_ref() != Some(&u);
                                    *g = Some(u.clone()); fresh };
                                if fresh {
                                    set_snapshot(|s| s.url = Some(u.clone()));
                                    // spec T1c：地址变化桌面通知（含首次拿到地址）
                                    crate::remote::events::emit_ui("remote-tunnel-address", serde_json::json!({"url": u}));
                                }
                            }
```

named 模式的 url：Token 对应子域 MAM 侧不可知（Cloudflare 面板配置），快照 url 置 None、设置页提示「地址以 Cloudflare 面板为准」——**修正**：named 也可从 stderr 解析连接日志里的子域（`INF +--------------------------------------------------------------------------------------------+` 上方会打印 hostname）。保守做法：named url 尝试从 stderr 中 `https://<sub>.<domain>` 行解析；解析不到留 None。复用：写 `parse_named_url(line) -> Option<String>`（含 `https://` 且非 trycloudflare 且非 docs.cloudflare.com 的 host 行）——测试一行即可。

`stop()` / `restart_if_running(port)`：

```rust
/// 停止隧道（幂等）：置 stop → kill 子进程由 supervisor wait 收敛 → abort 兜底 → 快照复位
pub fn stop() {
    if let Some(h) = TUNNEL.lock().unwrap().take() {
        h.stop.store(true, Ordering::Relaxed);
        h.supervisor.abort(); // kill 由 abort 传播（子进程 kill_on_drop 未设——补：spawn 时 .kill_on_drop(true)）
    }
    set_snapshot(|s| { s.mode = KEY_CHANNEL_VALUE_OFF.into(); s.url = None; s.error = None; });
}

/// 通道切换：运行中则重启（spec T1b「切换通道进程一并退出」）
pub fn restart_if_running(port: u16) {
    let running = TUNNEL.lock().unwrap().is_some();
    if running {
        stop();
        start_if_configured(port);
    }
}
```

（`cmd.spawn()` 前补 `.kill_on_drop(true)`，保证 supervisor abort 时子进程必死。）

`mod.rs` 接线：

1. `start_server()` 的 spawn 闭包内、`server::serve(...)` 调用**之前**加 `tunnel::start_if_configured(port);`（serve 失败退出任务时隧道留着无意义——改为 serve Err 分支补 `tunnel::stop();`）。
2. `stop_server()` 末尾加 `tunnel::stop();`。
3. `remote_status()` 追加三键 + addresses 首插：

```rust
    let tun = tunnel::snapshot();
    st["channel"] = serde_json::json!(crate::database::dao::settings::get_setting(KEY_CHANNEL).as_deref().unwrap_or("off"));
    st["tunnelUrl"] = serde_json::json!(tun.url);
    st["tunnelError"] = serde_json::json!(tun.error);
    st["addresses"] = serde_json::json!(address_entries_with_tunnel(
        tun.url.filter(|_| tun.error.is_none()),
        address_entries(&bind, port, &candidates),
    ));
```

4. 纯函数（可测核，`address_entries` 旁）：

```rust
/// 地址表隧道前插（纯函数，M4 T1a）：隧道地址恒首位 primary；既有条目补 kind="lan"
pub fn address_entries_with_tunnel(
    tunnel_url: Option<String>,
    mut base: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    for e in &mut base {
        if e.get("kind").is_none() { e["kind"] = serde_json::json!("lan"); }
        e["primary"] = serde_json::json!(false); // 隧道在时局域网不再推荐位
    }
    match tunnel_url {
        Some(u) => {
            let mut out = vec![serde_json::json!({
                "url": u, "iface": "", "primary": true, "kind": "tunnel",
            })];
            out.append(&mut base);
            out
        }
        None => {
            if let Some(first) = base.first_mut() { first["primary"] = serde_json::json!(true); }
            base
        }
    }
}

/// 配对 URL 隧道跟随（纯函数）：隧道开 → 隧道基址/m#token=
pub fn pair_url_with_tunnel(tunnel: Option<String>, base_url: String, token: &str) -> String {
    match tunnel {
        Some(t) => format!("{}/m#token={token}", t.trim_end_matches('/')),
        None => format!("{base_url}#token={token}"),
    }
}
```

5. `remote_issue_token()` 拼 URL 处改用 `pair_url_with_tunnel(tunnel::snapshot().url.filter(|_| tunnel::snapshot().error.is_none()), format!("http://{host}:{port}/m"), &token)`。
6. 新 IPC 命令（写设置 + 通道热切换 + 通知）：

```rust
/// 通道设置（M4 T1a）：写 KV；运行中热切换（restart_if_running）；成功后广播状态变更
#[tauri::command]
pub fn remote_set_channel(channel: String, token: Option<String>) -> Result<(), String> {
    let mode = tunnel::parse_channel(Some(&channel))
        .ok_or_else(|| format!("通道值非法: {channel}（仅 off/quick/named）"))?;
    crate::database::dao::settings::set_setting(KEY_CHANNEL, mode);
    if let Some(t) = token {
        crate::database::dao::settings::set_setting(KEY_TUNNEL_TOKEN, t.trim());
    }
    if mode != "off" && crate::database::dao::settings::get_setting(KEY_TUNNEL_TOKEN)
        .map(|v| v.trim().is_empty()).unwrap_or(true) && mode == "named" {
        return Err("命名隧道需要填写 Tunnel Token".into());
    }
    let (_, port) = bind_and_port().unwrap_or(("127.0.0.1".into(), DEFAULT_PORT));
    tunnel::restart_if_running(port);
    events::emit_ui("remote-changed", serde_json::json!({"channel": mode}));
    events::audit("channel_set", &format!("channel={mode}"));
    Ok(())
}
```

7. `remote_toggle` 成功后 emit：`toggle_core` 两分支成功路径后（`remote_toggle` 函数体内 `toggle_core(...)` 返回 Ok 时）`events::emit_ui("remote-changed", serde_json::json!({"enabled": enabled}));`

- [ ] **Step 4: 跑测试确认通过 + 门禁**

Run: `cd src-tauri && cargo test tunnel && cargo test pair_url && cargo test address_entries_with_tunnel && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全过；`dummy_child` 占位已随修正删除。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/remote/tunnel.rs src-tauri/src/remote/mod.rs
git commit -m "feat(m4-t1bc): 隧道进程管理——quick/named 双模式 spawn+stderr 地址解析+退避守护；status/issue_token/通道热切换接线"
```

---

### Task 6: T1a 前端 · 设置页通道区块 + 地址徽标 + Tailscale 指引

**Files:**
- Modify: `src/lib/api/remote.ts`（类型 + remoteSetChannel）
- Modify: `src/components/settings/RemoteSection.tsx`（通道区块 / tunnel 状态 / 地址徽标 / Tailscale 文案）
- Modify: `src/i18n/locales/zh.json`、`en.json`
- Modify: `src/tauri-mock.ts`（remote_set_channel case）
- Test: `tests/settings/remoteSection.test.tsx`

**Interfaces:**
- Consumes: Task 5 的 `remote_set_channel` / status 新键（channel/tunnelUrl/tunnelError/addresses[].kind）。
- Produces: `remote.ts` 的 `RemoteStatus.channel: string`、`tunnelUrl: string | null`、`tunnelError: string | null`、`RemoteAddressEntry.kind?: "tunnel" | "lan"`；`remoteSetChannel(channel, token?)`。

- [ ] **Step 1: 写失败测试**

```tsx
// M4 T1a：通道区块三选一 + named 需 Token + 隧道地址条目徽标
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
  // 隧道地址条目带「外部通道」徽标且居首
  expect(await screen.findByText(/external channel/i)).toBeInTheDocument();
  expect(screen.getByText("https://mam-mac.example.asia")).toBeInTheDocument();
  // 通道三按钮：named 高亮
  const namedBtn = screen.getByRole("button", { name: /named tunnel/i });
  expect(namedBtn).toBeInTheDocument();
  // 切到临时隧道 → invoke remote_set_channel
  fireEvent.click(screen.getByRole("button", { name: /quick tunnel/i }));
  await waitFor(() =>
    expect(invokeMock).toHaveBeenCalledWith("remote_set_channel", expect.objectContaining({ channel: "quick" }))
  );
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run tests/settings/remoteSection.test.tsx`
Expected: FAIL（无 external channel 文案）。

- [ ] **Step 3: 实现**

`remote.ts` 类型扩展 + 新函数：

```ts
export type RemoteStatus = {
  // ...既有字段...
  channel: string; // off / quick / named（M4 T1a）
  tunnelUrl: string | null;
  tunnelError: string | null;
};
export type RemoteAddressEntry = {
  url: string;
  iface: string;
  primary: boolean;
  kind?: "tunnel" | "lan"; // M4 T1a：tunnel=外部通道徽标（旧后端无此键 → undefined 按 lan 渲染）
};
export async function remoteSetChannel(channel: string, token?: string): Promise<void> {
  return await invoke("remote_set_channel", { channel, token: token ?? null });
}
```

`RemoteSection.tsx`：
1. 绑定区块之后插「外部通道」区块（结构对齐绑定二选：label + 三 Button + named 时的 Token Input）。组件内：

```tsx
const CHANNEL_OFF = "off", CHANNEL_QUICK = "quick", CHANNEL_NAMED = "named";
const channel = status?.channel ?? CHANNEL_OFF;
const [token, setToken] = useState("");
// 回填：进面板时读 KEY_TUNNEL_TOKEN
useEffect(() => {
  void (async () => setToken((await getSetting("remote.tunnel_token")) ?? ""))();
}, []);
const changeChannel = async (mode: string) => {
  if (mode === channel) return;
  try {
    await remoteSetChannel(mode, mode === CHANNEL_NAMED ? token : undefined);
    await refresh();
  } catch (e) {
    toast.error(formatInvokeError(e, t));
  }
};
const saveToken = async () => {
  try {
    await remoteSetChannel(CHANNEL_NAMED, token);
    await refresh();
    toast.success(t("settings.remote.channelSaved"));
  } catch (e) {
    toast.error(formatInvokeError(e, t));
  }
};
```

渲染（放「本机名」区块后）：

```tsx
{/* 外部通道（M4 T1a，2026-09-16 裁决：独立区块与绑定并存） */}
<div className="flex items-center justify-between gap-4 py-2.5">
  <div className="flex-1">
    <label className="text-sm font-medium">{t("settings.remote.channel")}</label>
    <p className="text-muted-foreground mt-0.5 text-xs">{t("settings.remote.channelHint")}</p>
  </div>
  <div className="flex gap-2">
    <Button variant={channel === CHANNEL_OFF ? "default" : "outline"} size="sm"
      onClick={() => void changeChannel(CHANNEL_OFF)}>{t("settings.remote.channelOff")}</Button>
    <Button variant={channel === CHANNEL_QUICK ? "default" : "outline"} size="sm"
      onClick={() => void changeChannel(CHANNEL_QUICK)}>{t("settings.remote.channelQuick")}</Button>
    <Button variant={channel === CHANNEL_NAMED ? "default" : "outline"} size="sm"
      onClick={() => void changeChannel(CHANNEL_NAMED)}>{t("settings.remote.channelNamed")}</Button>
  </div>
</div>
{channel !== CHANNEL_OFF && (
  <div className="border-t" />
)}
{channel === CHANNEL_NAMED && (
  <div className="flex items-center justify-between gap-4 py-2.5">
    <div className="flex-1">
      <label htmlFor="remote-tunnel-token" className="text-sm font-medium">{t("settings.remote.tunnelToken")}</label>
      <p className="text-muted-foreground mt-0.5 text-xs">{t("settings.remote.tunnelTokenHint")}</p>
    </div>
    <div className="flex w-72 gap-2">
      <Input id="remote-tunnel-token" value={token} type="password"
        placeholder="eyJh...（Cloudflare Tunnel Token）" className="flex-1"
        onChange={(e) => setToken(e.target.value)} />
      <Button size="sm" variant="outline" onClick={() => void saveToken()}>{t("settings.remote.channelSave")}</Button>
    </div>
  </div>
)}
{status?.tunnelError && (
  <p className="pb-2 text-xs text-amber-500">{status.tunnelError}</p>
)}
{status?.tunnelUrl && (
  <p className="text-muted-foreground pb-2 text-xs break-all">
    {t("settings.remote.tunnelCurrent")}: {status.tunnelUrl}
  </p>
)}
```

2. 地址条目徽标（`addresses.map` 内，iface 渲染前插）：

```tsx
{a.kind === "tunnel" && (
  <span className="rounded-full bg-blue-500/15 px-1.5 py-0.5 text-[10px] text-blue-600 dark:text-blue-400">
    {t("settings.remote.channelBadge")}
  </span>
)}
```

3. 底部安全警示下加 Tailscale 段：

```tsx
<p className="text-muted-foreground text-xs">{t("settings.remote.tailscaleHint")}</p>
```

i18n 新键（zh / en 成对，共 10 键）：`channel`外部通道/External Channel、`channelHint`「与「绑定」独立并存：隧道连本机回环，局域网与外网可同时用」/…、`channelOff`关闭/Off、`channelQuick`临时隧道/Quick Tunnel、`channelNamed`命名隧道/Named Tunnel、`tunnelToken`、`tunnelTokenHint`「Cloudflare Zero Trust → Networks → Tunnels 创建后复制的 Token」/…、`channelSave`保存/Save、`channelSaved`已保存/Saved、`tunnelCurrent`当前隧道地址/Current tunnel URL、`channelBadge`外部通道/External Channel、`tailscaleHint`「Tailscale 用户：在 Tailscale 网络内直接访问局域网地址即可，无需隧道（详见 Tailscale 文档）」/…。

`tauri-mock.ts`：switch 内 `case "remote_set_channel": return Promise.resolve(null);`（default 已兜 null，显式 case 便于 Playwright 场景扩展）。

- [ ] **Step 4: 跑测试 + 门禁**

Run: `pnpm vitest run tests/settings/remoteSection.test.tsx && pnpm check && pnpm build:mobile`
Expected: 全过（既有用例零回归）。

- [ ] **Step 5: 提交**

```bash
git add src/lib/api/remote.ts src/components/settings/RemoteSection.tsx src/i18n/locales/zh.json src/i18n/locales/en.json src/tauri-mock.ts tests/settings/remoteSection.test.tsx
git commit -m "feat(m4-t1a): 设置页外部通道区块（三选一+Token 输入+隧道状态）+ 地址「外部通道」徽标 + Tailscale 指引"
```

---

### Task 7: T2 后端 · 审批配对队列 + 设备上限 + 吊销/花名册命令

**Files:**
- Create: `src-tauri/src/remote/approval.rs`
- Modify: `src-tauri/src/remote/pairing.rs`（device_count / revoke_device / device_cookie）
- Modify: `src-tauri/src/remote/gate.rs`（放行名单扩 /pair/*）
- Modify: `src-tauri/src/remote/server.rs`（RemoteState.approval 字段 + 三个端点路由 + ConnectInfo）
- Modify: `src-tauri/src/remote/api.rs`（三端点 handler + api::pair 上限门）
- Modify: `src-tauri/src/remote/mod.rs`（新命令 ×5 + is_online 纯函数 + STATE 构造）
- Modify: `src-tauri/src/lib.rs`（generate_handler 注册 5 命令）

**Interfaces:**
- Produces（Rust）：`approval::ApprovalService{create,approve,poll,confirm,prune,pending}`；`pairing::device_cookie(id)->String`、`device_count(conn)->usize`、`revoke_device(conn,id)->Result<usize,String>`；命令 `remote_pending_requests`、`remote_approve_request(id)`、`remote_devices`、`remote_revoke_device(id)`、`remote_revoke_all_devices`；`mod::is_online(registry_hit,last_seen,now)->bool`；`mod::max_devices()->usize`（KEY_MAX_DEVICES="remote.max_devices"，默认 3，clamp 1..=10）。
- Produces（HTTP，均不过闸）：`POST /m/api/v1/pair/request {name}` → `200 {requestId,expiresAt}` | `429 {error:"queue_full"|"ip_busy"}`；`POST /m/api/v1/pair/poll {requestId}` → `200 {status:"pending"|"approved"|"expired", expiresAt}`（approved 附 Set-Cookie）；`POST /m/api/v1/pair/confirm {requestId,code}` → `200 {ok,error?,triesLeft?}`（ok 附 Set-Cookie；error ∈ wrong/exhausted/expired/not_found/cap_full）。
- Consumes: Task 1 `sse_registry`（吊销断连）、Task 3 `events::emit_ui/audit`。

- [ ] **Step 1: 写失败测试**

`approval.rs` 尾部：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> ApprovalService {
        ApprovalService::new(
            5 * 60 * 1000, // TTL 5 分钟（spec T2a）
            Box::new(|i| format!("r{i}")),
            Box::new(|| "1234".to_string()),
            Box::new(|i| format!("dev-{i}")),
        )
    }

    #[test]
    fn create_approve_poll_flow() {
        let mut s = svc();
        let r = s.create("我的手机", "UA", "1.1.1.1", 0).unwrap();
        assert_eq!(r.id, "r0");
        // 轮询：待批准
        assert!(matches!(s.poll("r0", 10), PollOutcome::Pending { .. }));
        // 桌面批准 → poll 拿设备 id（幂等：可重复 poll）
        assert_eq!(s.approve("r0", 20, || false), ApproveOutcome::Ok("dev-0".into()));
        assert_eq!(s.poll("r0", 21), PollOutcome::Approved("dev-0".into()));
        assert_eq!(s.poll("r0", 22), PollOutcome::Approved("dev-0".into()));
    }

    #[test]
    fn confirm_code_three_strikes() {
        let mut s = svc();
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert_eq!(s.confirm("r0", "0000", 10), ConfirmOutcome::Wrong(2));
        assert_eq!(s.confirm("r0", "0000", 11), ConfirmOutcome::Wrong(1));
        // 第 3 次错 → 作废（exhausted），正确码也不再用
        assert_eq!(s.confirm("r0", "0000", 12), ConfirmOutcome::Exhausted);
        assert_eq!(s.confirm("r0", "1234", 13), ConfirmOutcome::NotFound);
        // 正确路径
        s.create("d2", "UA", "2.2.2.2", 20).unwrap();
        assert_eq!(s.confirm("r1", "1234", 21), ConfirmOutcome::Ok("dev-1".into()));
    }

    #[test]
    fn ttl_expiry_and_prune() {
        let mut s = svc();
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert!(matches!(s.poll("r0", 5 * 60 * 1000 + 1), PollOutcome::Expired));
        // 过期项被 prune 后队列腾位
        assert_eq!(s.pending(5 * 60 * 1000 + 2).len(), 0);
    }

    #[test]
    fn queue_cap_and_ip_dedup() {
        let mut s = svc();
        for i in 0..3 {
            s.create(&format!("d{i}"), "UA", &format!("10.0.0.{i}"), 0).unwrap();
        }
        assert_eq!(s.create("d3", "UA", "10.0.0.9", 0), Err(CreateRejection::QueueFull));
        // 同 IP 第二个请求被拒（即使队列未满）
        let mut s2 = svc();
        s2.create("a", "UA", "3.3.3.3", 0).unwrap();
        assert_eq!(s2.create("b", "UA", "3.3.3.3", 1), Err(CreateRejection::IpBusy));
    }

    #[test]
    fn approve_respects_cap_full() {
        let mut s = svc();
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert_eq!(s.approve("r0", 0, || true), ApproveOutcome::CapFull);
        assert_eq!(s.approve("r0", 1, || false), ApproveOutcome::Ok("dev-0".into()));
    }
}
```

`server.rs` tests 追加端点集成（沿 `test_state()`，需在 `test_state` 里构造 `approval` 字段）：

```rust
    // ---- 审批配对端到端（M4 T2）----

    fn state_with_approval() -> Arc<RemoteState> {
        let mut arc = test_state();
        // Arc 唯一引用期原地换 approval：假时钟 now=1000、4 位码恒 2468
        // （Arc::get_mut：test_state 刚构造、无其他引用，必然 Some）
        std::sync::Arc::get_mut(&mut arc).unwrap().approval =
            std::sync::Mutex::new(crate::remote::approval::ApprovalService::new(
                300_000,
                Box::new(|i| format!("req-{i}")),
                Box::new(|| "2468".to_string()),
                Box::new(|i| format!("adev-{i}")),
            ));
        arc
    }

    #[tokio::test]
    async fn approval_request_poll_confirm_endpoints() {
        let state = state_with_approval();
        let app = super::router_with_static(state.clone());
        // 1. 请求接入（不携带 cookie——gate 放行 /pair/*）
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/m/api/v1/pair/request")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"name":"我的手机"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["requestId"], "req-0");
        // 2. poll：pending
        let resp = app.clone().oneshot(post_json("/m/api/v1/pair/poll", r#"{"requestId":"req-0"}"#)).await.unwrap();
        assert_eq!(resp.status(), 200);
        // 3. confirm 错码 → wrong + triesLeft；正码 → Set-Cookie
        let resp = app.clone().oneshot(post_json("/m/api/v1/pair/confirm", r#"{"requestId":"req-0","code":"0000"}"#)).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], serde_json::json!(false));
        assert_eq!(v["error"], serde_json::json!("wrong"));
        let resp = app.clone().oneshot(post_json("/m/api/v1/pair/confirm", r#"{"requestId":"req-0","code":"2468"}"#)).await.unwrap();
        let cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap().to_string();
        assert!(cookie.contains("mam_device=adev-0"));
        // 4. 直通配对上限门：预置 3 台有效设备 → api::pair 拒绝（复用既有 token 流程断言 403 + body error）
    }

    fn post_json(uri: &str, body: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }
```

`mod.rs` tests 追加纯函数：

```rust
    #[test]
    fn online_threshold_30s() {
        assert!(is_online(true, 0, 60_000));  // 活跃 SSE 连接：不看 last_seen
        assert!(is_online(false, 40_000, 60_000)); // 20s 前过闸 → 在线
        assert!(!is_online(false, 20_000, 60_000)); // 40s 前过闸 → 离线
    }

    #[test]
    fn max_devices_default_and_clamp() {
        assert_eq!(max_devices_from(None), 3);
        assert_eq!(max_devices_from(Some("1".into())), 1);
        assert_eq!(max_devices_from(Some("99".into())), 10); // clamp 上限
        assert_eq!(max_devices_from(Some("x".into())), 3);   // 乱串回落默认
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test approval && cargo test online_ && cargo test max_devices`
Expected: 编译失败。

- [ ] **Step 3: 实现**

`approval.rs`：

```rust
// 审批配对队列（M4 T2a/T2b/T2c）：内存态状态机，重启即清（spec T2a 边界——请求不落库）。
// 设备上限不在本层判（approve/confirm 的调用方注入 cap 闭包——三入口统一门，spec T2c）

/// 队列并发上限（spec T2a 自决默认）
pub const MAX_QUEUE: usize = 3;
/// 请求有效期（spec T2a：5 分钟）
pub const REQUEST_TTL_MS: i64 = 5 * 60 * 1000;
/// 4 位码限试次数（spec T2b）
pub const MAX_CODE_TRIES: u8 = 3;

pub struct ApprovalRequest {
    pub id: String,
    pub name: String,
    pub ua: String,
    pub ip: String,
    pub code: String,
    pub tries: u8,
    pub approved_device: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, PartialEq)]
pub enum CreateRejection { QueueFull, IpBusy }

#[derive(Debug, PartialEq)]
pub enum ApproveOutcome {
    Ok(String),
    NotFound,
    Expired,
    CapFull,
}

#[derive(Debug, PartialEq)]
pub enum PollOutcome {
    Pending { expires_at: i64 },
    Approved(String),
    Expired,
}

#[derive(Debug, PartialEq)]
pub enum ConfirmOutcome {
    Ok(String),
    Wrong(u8), // 剩余次数
    Exhausted,
    Expired,
    NotFound,
}

pub struct ApprovalService {
    ttl_ms: i64,
    id_gen: Box<dyn Fn(usize) -> String + Send + Sync>,
    code_gen: Box<dyn Fn() -> String + Send + Sync>,
    device_gen: Box<dyn Fn(usize) -> String + Send + Sync>,
    requests: Vec<ApprovalRequest>,
}

impl ApprovalService {
    pub fn new(
        ttl_ms: i64,
        id_gen: Box<dyn Fn(usize) -> String + Send + Sync>,
        code_gen: Box<dyn Fn() -> String + Send + Sync>,
        device_gen: Box<dyn Fn(usize) -> String + Send + Sync>,
    ) -> Self {
        Self { ttl_ms, id_gen, code_gen, device_gen, requests: Vec::new() }
    }

    fn prune(&mut self, now: i64) {
        self.requests.retain(|r| r.expires_at > now);
    }

    /// 新请求：TTL 清理 → 同 IP 占位检查 → 队列上限检查
    pub fn create(&mut self, name: &str, ua: &str, ip: &str, now: i64) -> Result<&ApprovalRequest, CreateRejection> {
        self.prune(now);
        if self.requests.iter().any(|r| r.ip == ip) {
            return Err(CreateRejection::IpBusy);
        }
        if self.requests.len() >= MAX_QUEUE {
            return Err(CreateRejection::QueueFull);
        }
        let id = (self.id_gen)(self.requests.len());
        self.requests.push(ApprovalRequest {
            id: id.clone(),
            name: name.trim().chars().take(40).collect(),
            ua: ua.chars().take(200).collect(),
            ip: ip.to_string(),
            code: (self.code_gen)(),
            tries: 0,
            approved_device: None,
            created_at: now,
            expires_at: now + self.ttl_ms,
        });
        Ok(self.requests.last().unwrap())
    }

    /// 桌面批准（cap 闭包 = 满员判定，三入口同门）
    pub fn approve(&mut self, id: &str, now: i64, cap_exceeded: impl FnOnce() -> bool) -> ApproveOutcome {
        self.prune(now);
        let Some(r) = self.requests.iter_mut().find(|r| r.id == id) else {
            return ApproveOutcome::NotFound;
        };
        let device = (self.device_gen)(self.requests.iter().position(|x| x.id == id).unwrap_or(0));
        if cap_exceeded() {
            return ApproveOutcome::CapFull;
        }
        r.approved_device = Some(device.clone());
        ApproveOutcome::Ok(device)
    }

    pub fn poll(&mut self, id: &str, now: i64) -> PollOutcome {
        self.prune(now);
        match self.requests.iter().find(|r| r.id == id) {
            None => PollOutcome::Expired, // 作废/不存在对手机同观感（不给预言机）
            Some(r) => match &r.approved_device {
                Some(d) => PollOutcome::Approved(d.clone()),
                None => PollOutcome::Pending { expires_at: r.expires_at },
            },
        }
    }

    /// 4 位码确认（cap 由调用方在 Ok 前置检查——满员时码对了也拒）
    pub fn confirm(&mut self, id: &str, code: &str, now: i64) -> ConfirmOutcome {
        self.prune(now);
        let Some(pos) = self.requests.iter().position(|r| r.id == id) else {
            return ConfirmOutcome::NotFound;
        };
        let r = &mut self.requests[pos];
        if r.approved_device.is_some() {
            return ConfirmOutcome::Ok(r.approved_device.clone().unwrap());
        }
        if r.code == code.trim() {
            let device = (self.device_gen)(pos);
            r.approved_device = Some(device.clone());
            return ConfirmOutcome::Ok(device);
        }
        r.tries += 1;
        if r.tries >= MAX_CODE_TRIES {
            self.requests.remove(pos); // 错满 3 次：作废需重新发起（spec T2b）
            ConfirmOutcome::Exhausted
        } else {
            ConfirmOutcome::Wrong(MAX_CODE_TRIES - r.tries)
        }
    }

    /// 桌面面板列表（含过期项剔除）
    pub fn pending(&mut self, now: i64) -> Vec<ApprovalRequest> {
        self.prune(now);
        self.requests.clone()
    }
}
```

（`ApprovalRequest` 需 `#[derive(Clone)]`。）

`pairing.rs` 追加三函数 + cookie 拼装收口：

```rust
/// 有效设备数（上限门计数口径：revoked=0）
pub fn device_count(conn: &rusqlite::Connection) -> usize {
    conn.query_row("SELECT COUNT(*) FROM remote_devices WHERE revoked = 0", [], |r| r.get::<_, i64>(0))
        .map(|n| n as usize)
        .unwrap_or(0)
}

/// 单设备吊销（返回影响行数：0 = 本就无效/不存在）
pub fn revoke_device(conn: &rusqlite::Connection, device_id: &str) -> Result<usize, String> {
    conn.execute("UPDATE remote_devices SET revoked = 1 WHERE id = ?1 AND revoked = 0", [device_id])
        .map_err(|e| format!("吊销设备失败: {e}"))
}

/// 设备 cookie 统一拼装（api::pair / pair-poll / pair-confirm 三处共用，防漂移）
pub fn device_cookie(device_id: &str) -> String {
    format!(
        "{COOKIE}={device_id}; Path=/m; HttpOnly; SameSite=Lax; Max-Age={}",
        DEVICE_TTL_MS / 1000
    )
}
// COOKIE 常量 = gate::COOKIE_NAME —— pairing.rs 顶部 use crate::remote::gate::COOKIE_NAME as COOKIE;
```

（`api::pair` 原 cookie 拼装改调 `device_cookie(&device_id)`。）

`gate.rs` 放行名单：

```rust
    // M4 T2：审批配对三端点（/pair/request|poll|confirm）与 /pair 同为换 cookie 入口，放行
    if path == "/pair" || path.starts_with("/pair/") {
        return next.run(req).await;
    }
```

`server.rs`：
1. `RemoteState` 加 `pub approval: std::sync::Mutex<super::approval::ApprovalService>,`
2. `api_router` 追加路由：

```rust
        .route("/pair/request", post(api::pair_request))
        .route("/pair/poll", post(api::pair_poll))
        .route("/pair/confirm", post(api::pair_confirm))
```

3. `serve()` 尾部改带 ConnectInfo（审批请求记来源 IP）：

```rust
    axum::serve(
        listener,
        router_with_static(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
```

`api.rs` 三 handler + pair 上限门：

```rust
#[derive(Deserialize)]
pub struct PairRequestBody { pub name: Option<String> }

/// POST /m/api/v1/pair/request（M4 T2a）：新设备请求接入（未过闸端点）。
/// 成功即桌面通知（emit remote-pair-request）+ 审计留痕
pub async fn pair_request(
    State(st): State<Arc<RemoteState>>,
    ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PairRequestBody>,
) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let ua = headers.get(axum::http::header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let ip = addr.ip().to_string();
    let name = req.name.unwrap_or_default();
    let mut svc = st.approval.lock().unwrap();
    match svc.create(&name, &ua, &ip, now) {
        Ok(r) => {
            super::events::emit_ui("remote-pair-request", serde_json::json!({
                "name": r.name, "ip": r.ip, "expiresAt": r.expires_at,
            }));
            super::events::audit("pair_request", &format!("id={} name={} ip={}", r.id, r.name, r.ip));
            Json(serde_json::json!({ "requestId": r.id, "expiresAt": r.expires_at })).into_response()
        }
        Err(e) => {
            let err = match e { CreateRejection::QueueFull => "queue_full", CreateRejection::IpBusy => "ip_busy" };
            super::events::audit("pair_request_rejected", &format!("ip={ip} reason={err}"));
            (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({ "error": err }))).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct PairPollBody { pub request_id: String }

/// POST /m/api/v1/pair/poll：手机轮询审批结果（approved 幂等重发 Set-Cookie——丢包重 poll 不掉凭证）
pub async fn pair_poll(State(st): State<Arc<RemoteState>>, Json(req): Json<PairPollBody>) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let outcome = st.approval.lock().unwrap().poll(&req.request_id, now);
    match outcome {
        PollOutcome::Approved(device) => {
            super::events::audit("pair_polled", &format!("device={device}"));
            let cookie = crate::remote::pairing::device_cookie(&device);
            st.store.with(|c| { let _ = crate::remote::pairing::touch_device(c, &device, now); });
            ([(axum::http::header::SET_COOKIE, cookie)],
             Json(serde_json::json!({ "status": "approved" }))).into_response()
        }
        PollOutcome::Pending { expires_at } =>
            Json(serde_json::json!({ "status": "pending", "expiresAt": expires_at })).into_response(),
        PollOutcome::Expired =>
            Json(serde_json::json!({ "status": "expired" })).into_response(),
    }
}

#[derive(Deserialize)]
pub struct PairConfirmBody { pub request_id: String, pub code: String }

/// POST /m/api/v1/pair/confirm：4 位确认码等效授权（限试 3 次，spec T2b）
pub async fn pair_confirm(State(st): State<Arc<RemoteState>>, Json(req): Json<PairConfirmBody>) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let max = super::max_devices();
    let store = st.store.clone();
    let outcome = st.approval.lock().unwrap().confirm(&req.request_id, &req.code, now);
    // Ok 路径补上限门 + 落库（confirm 内部不触 DB——状态机纯内存）
    match outcome {
        ConfirmOutcome::Ok(device) => {
            let cap = store.with(|c| crate::remote::pairing::device_count(c) >= max);
            if cap {
                return Json(serde_json::json!({ "ok": false, "error": "cap_full" })).into_response();
            }
            super::events::audit("pair_confirmed", &format!("device={device}"));
            persist_and_cookie(&st, &device, now)
        }
        ConfirmOutcome::Wrong(left) =>
            Json(serde_json::json!({ "ok": false, "error": "wrong", "triesLeft": left })).into_response(),
        ConfirmOutcome::Exhausted => {
            super::events::audit("pair_confirm_exhausted", &req.request_id);
            Json(serde_json::json!({ "ok": false, "error": "exhausted" })).into_response()
        }
        ConfirmOutcome::Expired | ConfirmOutcome::NotFound =>
            Json(serde_json::json!({ "ok": false, "error": "expired" })).into_response(),
    }
}

/// 批准/确认通过的公共落库 + 下发 cookie（与 api::pair 同形设备记录）
fn persist_and_cookie(st: &Arc<RemoteState>, device_id: &str, now: i64) -> Response {
    let dev = crate::remote::pairing::NewDevice {
        id: device_id.to_string(),
        name: String::new(),
        ua: String::new(),
        origin_ip: String::new(),
        paired_at: now,
    };
    st.store.with(|c| { let _ = crate::remote::pairing::persist_device(c, &dev); });
    (
        [(axum::http::header::SET_COOKIE, crate::remote::pairing::device_cookie(device_id))],
        Json(serde_json::json!({ "ok": true })),
    ).into_response()
}
```

（poll 的 Approved 路径同样要 persist——`poll` 里 Approved 分支改调 `persist_and_cookie(&st, &device, now)`，cookie/落库同源；测试里 poll 后设备即入库。）

`api::pair` 上限门（`svc.accept` 之前插入）：

```rust
    // M4 T2c：上限三入口统一门——直通扫码也拒（spec：否则扫码绕过上限）
    let max = super::max_devices();
    if st.store.with(|c| crate::remote::pairing::device_count(c) >= max) {
        super::events::audit("pair_rejected_cap", &format!("max={max}"));
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "cap_full" })),
        ).into_response();
    }
```

（注意：上限门在 accept **之前**——token 不被消费，腾位后原码仍可用。）

`mod.rs`：

```rust
/// 设备上限键（spec T2c：默认 3 台可配）
pub const KEY_MAX_DEVICES: &str = "remote.max_devices";

/// 上限解析（纯函数）：None/乱串 → 3；clamp 1..=10
pub fn max_devices_from(v: Option<String>) -> usize {
    v.and_then(|s| s.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, 10))
        .unwrap_or(3)
}
pub fn max_devices() -> usize {
    max_devices_from(crate::database::dao::settings::get_setting(KEY_MAX_DEVICES))
}

/// 在线口径（spec T2d 自审修正）：活跃 SSE 连接 ∨ 30s 内过闸
pub fn is_online(registry_hit: bool, last_seen_at: i64, now: i64) -> bool {
    registry_hit || now - last_seen_at < 30_000
}
```

STATE 构造追加 `approval: Mutex::new(approval::ApprovalService::new(5*60*1000, 实时钟/码/设备生成))`（id/设备生成复用 pairing 的 16 字节 hex 风格；code 生成 `format!("{:04}", rand::thread_rng().gen_range(0..10000))`）。

5 个 IPC 命令（`remote_issue_token` 之后）：

```rust
/// 待审批队列（设置页面板数据源；name/ip/ua/expiresAt/4位码）
#[tauri::command]
pub fn remote_pending_requests() -> serde_json::Value {
    let now = chrono::Utc::now().timestamp_millis();
    let list = STATE.approval.lock().unwrap().pending(now);
    serde_json::json!(list.iter().map(|r| serde_json::json!({
        "id": r.id, "name": r.name, "ip": r.ip, "ua": r.ua,
        "code": r.code, "expiresAt": r.expires_at,
    })).collect::<Vec<_>>())
}

/// 桌面批准（spec T2b 路径一）
#[tauri::command]
pub fn remote_approve_request(id: String) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let max = max_devices();
    let cap = STATE.store.with(|c| pairing::device_count(c) >= max);
    match STATE.approval.lock().unwrap().approve(&id, now, || cap) {
        approval::ApproveOutcome::Ok(_) => { events::audit("pair_approved", &id); Ok(()) }
        approval::ApproveOutcome::CapFull => Err("设备已满，请先在花名册吊销腾位".into()),
        approval::ApproveOutcome::NotFound | approval::ApproveOutcome::Expired => Err("请求已过期或不存在".into()),
    }
}

/// 设备花名册（在线口径 = SSE 注册表 ∨ 30s 过闸）
#[tauri::command]
pub fn remote_devices() -> serde_json::Value {
    let now = chrono::Utc::now().timestamp_millis();
    let rows: Vec<(String, String, i64, i64, i64)> = STATE.store.with(|c| {
        c.prepare("SELECT id, name, first_paired_at, last_seen_at, revoked FROM remote_devices ORDER BY first_paired_at")
            .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    });
    serde_json::json!(rows.iter().filter(|(_, _, _, _, revoked)| *revoked == 0).map(|(id, name, paired, seen, _)| {
        serde_json::json!({
            "id": id, "name": name, "firstPairedAt": paired,
            "lastSeenAt": seen, "online": is_online(STATE.sse_registry.has(id), *seen, now),
        })
    }).collect::<Vec<_>>())
}
```

（**name 取回**：设备表 name 列在直通/审批落库时均为空——M4 一并修：`NewDevice` 落库时 name 写入请求里的设备名/「直通扫码」；`persist_and_cookie` 与 `api::pair` 传名。`remote_devices` SELECT 补 name 列透出。）

```rust
/// 单设备吊销：DB 置位 + SSE 即时断连（Task 1 注册表接线）+ 审计
#[tauri::command]
pub fn remote_revoke_device(id: String) -> Result<(), String> {
    STATE.store.with(|c| pairing::revoke_device(c, &id))?;
    let n = STATE.sse_registry.disconnect_device(&id);
    events::audit("device_revoked", &format!("id={id} closed_sse={n}"));
    events::emit_ui("remote-roster-changed", serde_json::json!({"id": id}));
    Ok(())
}

/// 全部吊销（不停止服务器——与「停止远程」的差别只在服务存续）
#[tauri::command]
pub fn remote_revoke_all_devices() -> Result<usize, String> {
    let n = STATE.store.with(|c| pairing::revoke_all(c)).map_err(|e| e.to_string())?;
    let closed = STATE.sse_registry.disconnect_all();
    events::audit("devices_revoked_all", &format!("count={n} closed_sse={closed}"));
    events::emit_ui("remote-roster-changed", serde_json::json!({}));
    Ok(n)
}
```

`lib.rs` generate_handler 追加：`remote::remote_pending_requests, remote::remote_approve_request, remote::remote_devices, remote::remote_revoke_device, remote::remote_revoke_all_devices, remote::remote_set_channel,`（set_channel 属 Task 5，此处一并注册）。

- [ ] **Step 4: 跑测试确认通过 + 门禁**

Run: `cd src-tauri && cargo test approval && cargo test online_ && cargo test max_devices && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全部（含既有 gate/pair/server 回归）绿。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/remote/approval.rs src-tauri/src/remote/pairing.rs src-tauri/src/remote/gate.rs src-tauri/src/remote/server.rs src-tauri/src/remote/api.rs src-tauri/src/remote/mod.rs src-tauri/src/lib.rs
git commit -m "feat(m4-t2): 审批配对后端——队列状态机(TTL/上限/同IP/限试)+三端点+设备上限三入口同门+吊销/花名册命令+审计留痕"
```

---

### Task 8: T2 前端桌面 · 待审批面板 + 花名册 + 配对通知

**Files:**
- Modify: `src/lib/api/remote.ts`（5 个 IPC 封装 + 类型）
- Modify: `src/components/settings/RemoteSection.tsx`（两个新区块）
- Modify: `src/main.tsx`（AppWrapper 挂事件监听 hook）
- Create: `src/hooks/useRemoteEvents.ts`
- Modify: `src/i18n/locales/zh.json`、`en.json`（~20 键）
- Test: `tests/settings/remoteSection.test.tsx`

**Interfaces:**
- Consumes: Task 7 全部命令 + 事件（`remote-pair-request` / `remote-roster-changed`）。
- Produces: `remote.ts` 的 `remotePendingRequests()/remoteApproveRequest(id)/remoteDevices()/remoteRevokeDevice(id)/remoteRevokeAllDevices()` + `PendingRequest`/`RemoteDevice` 类型；`useRemoteEvents()` hook（系统通知 + 托盘事件路由，Task 11 复用）。

- [ ] **Step 1: 写失败测试**

```tsx
// M4 T2：待审批面板（4 位码可见 + 批准）与花名册（在线点 + 吊销）
it("pairing panel: pending request shows code, approve works; roster revokes", async () => {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "remote_status") return Promise.resolve({ ...lanStatus, enabled: true });
    if (cmd === "get_setting") return Promise.resolve(null);
    if (cmd === "remote_pending_requests")
      return Promise.resolve([
        { id: "r0", name: "我的手机", ip: "192.168.1.9", ua: "Mobile Safari", code: "2468", expiresAt: Date.now() + 300_000 },
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
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run tests/settings/remoteSection.test.tsx` → FAIL。

- [ ] **Step 3: 实现**

`remote.ts` 追加：

```ts
export type PendingRequest = {
  id: string; name: string; ip: string; ua: string;
  code: string; expiresAt: number;
};
export type RemoteDevice = {
  id: string; name: string; firstPairedAt: number;
  lastSeenAt: number; online: boolean;
};
export async function remotePendingRequests(): Promise<PendingRequest[]> {
  return await invoke<PendingRequest[]>("remote_pending_requests");
}
export async function remoteApproveRequest(id: string): Promise<void> {
  return await invoke("remote_approve_request", { id });
}
export async function remoteDevices(): Promise<RemoteDevice[]> {
  return await invoke<RemoteDevice[]>("remote_devices");
}
export async function remoteRevokeDevice(id: string): Promise<void> {
  return await invoke("remote_revoke_device", { id });
}
export async function remoteRevokeAllDevices(): Promise<void> {
  return await invoke("remote_revoke_all_devices");
}
```

**注意**：Rust 端 `remote_pending_requests`/`remote_devices` 返回 `serde_json::Value`（数组形态）——invoke 反序列化为数组即可；若空态为 `[]` 无问题。

`RemoteSection.tsx`：
1. 状态与轮询（enabled 时 3s 拉两列表——与移动端看板同节奏；页面不可见时暂停可不做，设置页开着一览无余）：

```tsx
const [pendingReqs, setPendingReqs] = useState<PendingRequest[]>([]);
const [devices, setDevices] = useState<RemoteDevice[]>([]);
useEffect(() => {
  if (!enabled) { setPendingReqs([]); setDevices([]); return; }
  const load = async () => {
    try {
      setPendingReqs(await remotePendingRequests());
      setDevices(await remoteDevices());
    } catch { /* 面板刷新尽力而为 */ }
  };
  void load();
  const timer = setInterval(() => void load(), 3000);
  return () => clearInterval(timer);
}, [enabled]);
```

2. 「配对面板」区块（二维码区块后）：pendingReqs 列表渲染（name/ip/ua 截断/剩余秒 `Math.max(0, Math.ceil((expiresAt - Date.now()) / 1000))`/code 大字/批准按钮 → `remoteApproveRequest(r.id)` 成功即刷新 + toast，Err toast 原文——CapFull 文案直接来自后端）。
3. 「设备花名册」区块（配对面板后）：devices 列表（name 或 id 前 8 位/在线绿点 `bg-green-500` : `bg-gray-400`/最近活跃相对时间）+ 每行吊销按钮 + 底部「全部吊销」+ 上限数字输入（`getSetting/setSetting("remote.max_devices")`，blur 落盘，`Number` clamp 前端不判——后端 `max_devices_from` 是唯一口径）。
4. `approve/revoke` 全部走 `toast.error(formatInvokeError(e, t))` 失败路径。

`useRemoteEvents.ts`（桌面全局事件 → 系统通知/剪贴板；`main.tsx` AppWrapper 的 useEffect 里调用一次）：

```ts
// 桌面全局远程事件（M4）：配对请求系统通知（点击直达设置页）/ 隧道地址变化通知 /
// 托盘触发的地址复制与开关失败 toast。main.tsx AppWrapper 挂载一次。
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { sendNotification, isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { toast } from "sonner";

export function useRemoteEvents() {
  useEffect(() => {
    const unlisten: Array<() => void> = [];
    void (async () => {
      unlisten.push(await listen<{ name: string; ip: string }>("remote-pair-request", async (e) => {
        let ok = await isPermissionGranted();
        if (!ok) ok = (await requestPermission()) === "granted";
        if (ok) {
          sendNotification({
            title: "MAM 配对请求",
            body: `${e.payload.name || "新设备"}（${e.payload.ip}）请求接入，点击前往审批`,
          });
        }
        toast.info("收到配对请求，请到 设置 → 远程接入 审批");
      }));
      unlisten.push(await listen<{ url: string }>("remote-tunnel-address", (e) => {
        toast.success(`外部地址已就绪: ${e.payload.url}`);
      }));
      unlisten.push(await listen<{ url: string }>("remote-copy-addr", async (e) => {
        await navigator.clipboard.writeText(e.payload.url);
        toast.success("已复制远程地址");
      }));
      unlisten.push(await listen<{ error: string }>("remote-toggle-failed", (e) => {
        toast.error(`远程开关失败: ${e.payload.error}`);
      }));
    })();
    return () => unlisten.forEach((f) => f());
  }, []);
}
```

（通知点击直达设置页：plugin-notification 的点击事件在无 actionTypeId 时只聚焦应用——本期以「通知 + toast 指路」交付，deep-link 登记为验收观察项，不阻塞。）

i18n 键（zh/en 成对，命名 `settings.remote.pending*` / `roster*`：pendingTitle 待审批设备/Pending devices、pendingEmpty 暂无请求/No pending requests、pendingApprove 批准/Approve、rosterTitle 设备花名册/Device roster、rosterEmpty 尚无已配对设备/No paired devices、rosterRevoke 吊销/Revoke、rosterRevokeAll 全部吊销/Revoke all、rosterOnline 在线/Online、rosterOffline 离线/Offline、rosterMax 设备上限/Device limit、pairNotifyTitle/body 等——以实现时实际文案为准成对补齐）。

- [ ] **Step 4: 跑测试 + 门禁**

Run: `pnpm vitest run tests/settings/remoteSection.test.tsx && pnpm check`
Expected: 全过。

- [ ] **Step 5: 提交**

```bash
git add src/lib/api/remote.ts src/components/settings/RemoteSection.tsx src/hooks/useRemoteEvents.ts src/main.tsx src/i18n/locales/zh.json src/i18n/locales/en.json tests/settings/remoteSection.test.tsx
git commit -m "feat(m4-t2): 桌面配对面板（待审批+4位码+批准）与设备花名册（在线点/吊销/上限）+ 配对请求系统通知"
```

---

### Task 9: T2 前端移动 · PairPage 请求接入流

**Files:**
- Modify: `src/mobile/api.ts`（requestPairing/pollPairing/confirmPairing + ApiError 细化）
- Modify: `src/mobile/PairPage.tsx`（请求接入子视图 + 4 位码输入 + 轮询）
- Test: `tests/mobile/api.test.ts`、`tests/mobile/App.test.tsx`（或新建 `PairRequest.test.tsx`）

**Interfaces:**
- Consumes: Task 7 三端点。
- Produces: `api.ts` 的 `requestPairing(name): Promise<{requestId; expiresAt}>`（429 抛 `ApiError(429, "queue_full"|"ip_busy")`）、`pollPairing(id): Promise<"pending"|"approved"|"expired">`（approved 时服务端已 Set-Cookie）、`confirmPairing(id, code): Promise<{ok: boolean; error?: string; triesLeft?: number}>`。

- [ ] **Step 1: 写失败测试**

`tests/mobile/api.test.ts` 追加（沿用文件内 fetch mock 先例）：

```ts
// M4 T2：请求接入三端点封装
it("requestPairing parses 200 and maps 429", async () => {
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
  fetchMock.mockResolvedValueOnce(new Response('{"requestId":"r0","expiresAt":1}', { status: 200 }));
  const r = await requestPairing("我的手机");
  expect(r.requestId).toBe("r0");
  fetchMock.mockResolvedValueOnce(new Response('{"error":"queue_full"}', { status: 429 }));
  await expect(requestPairing("x")).rejects.toMatchObject({ status: 429, message: "queue_full" });
});

it("pollPairing returns status strings", async () => {
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
  fetchMock.mockResolvedValueOnce(new Response('{"status":"pending","expiresAt":1}', { status: 200 }));
  expect(await pollPairing("r0")).toBe("pending");
  fetchMock.mockResolvedValueOnce(new Response('{"status":"approved"}', { status: 200 }));
  expect(await pollPairing("r0")).toBe("approved");
});

it("confirmPairing surfaces wrong + triesLeft", async () => {
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
  fetchMock.mockResolvedValueOnce(new Response('{"ok":false,"error":"wrong","triesLeft":2}', { status: 200 }));
  expect(await confirmPairing("r0", "0000")).toEqual({ ok: false, error: "wrong", triesLeft: 2 });
});
```

`tests/mobile/` 新建 `PairRequest.test.tsx`（渲染 PairPage，mock `./api`，走请求→pending→approved 状态机断言 onPaired 调用）。

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm vitest run tests/mobile/api.test.ts tests/mobile/PairRequest.test.tsx` → FAIL（函数不存在）。

- [ ] **Step 3: 实现**

`api.ts` 追加：

```ts
export interface PairRequestResult { requestId: string; expiresAt: number }

/** 请求接入（M4 T2）：429（队列满/同 IP 占位）抛 ApiError，message 即 error 串 */
export async function requestPairing(name: string): Promise<PairRequestResult> {
  const r = await fetch("/m/api/v1/pair/request", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ name }),
  });
  if (r.status === 429) {
    const v = (await r.json()) as { error: string };
    throw new ApiError(429, v.error ?? "busy");
  }
  if (!r.ok) throw new ApiError(r.status, "request failed");
  return (await r.json()) as PairRequestResult;
}

export type PairPollStatus = "pending" | "approved" | "expired";
export async function pollPairing(requestId: string): Promise<PairPollStatus> {
  const r = await fetch("/m/api/v1/pair/poll", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ requestId }),
  });
  const v = (await r.json()) as { status: PairPollStatus };
  return v.status;
}

export interface ConfirmResult { ok: boolean; error?: string; triesLeft?: number }
export async function confirmPairing(requestId: string, code: string): Promise<ConfirmResult> {
  const r = await fetch("/m/api/v1/pair/confirm", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ requestId, code }),
  });
  return (await r.json()) as ConfirmResult;
}
```

`PairPage.tsx` 改造（保留原 token 输入为主视图，下方加请求接入区）：

```tsx
type RequestState = "idle" | "waiting" | "denied";
const [reqState, setReqState] = useState<RequestState>("idle");
const [reqId, setReqId] = useState<string | null>(null);
const [deviceName, setDeviceName] = useState("");
const [code, setCode] = useState("");
const [msg, setMsg] = useState(""); // 错误/提示行（设备已满/次数用尽/过期）

const startRequest = async () => {
  try {
    const r = await requestPairing(deviceName || "手机浏览器");
    setReqId(r.requestId);
    setReqState("waiting");
    setMsg("");
  } catch (e) {
    // 429 queue_full/ip_busy → 同一友好文案（不给队列状态预言机）
    setMsg(e instanceof ApiError && e.status === 429 ? "请求过多，请稍后再试" : "请求失败，请重试");
  }
};

const submitCode = async () => {
  if (!reqId || code.trim().length !== 4) return;
  const r = await confirmPairing(reqId, code.trim());
  if (r.ok) { onPaired(); return; }
  if (r.error === "cap_full") setMsg("设备已满，请在桌面端花名册腾位后重试");
  else if (r.error === "exhausted") { setReqState("idle"); setMsg("错误次数过多，请重新发起请求"); }
  else if (r.error === "expired") { setReqState("idle"); setMsg("请求已过期，请重新发起"); }
  else setMsg(`确认码错误，剩余 ${r.triesLeft ?? 0} 次机会`);
};

// waiting 态轮询（3s；卸载即停）
useEffect(() => {
  if (reqState !== "waiting" || !reqId) return;
  let stop = false;
  const tick = async () => {
    try {
      const s = await pollPairing(reqId!);
      if (stop) return;
      if (s === "approved") { onPaired(); return; }
      if (s === "expired") { if (!stop) { setReqState("idle"); setMsg("请求已过期（5 分钟），请重新发起"); } }
    } catch { /* 网络抖动继续轮询 */ }
  };
  void tick();
  const timer = setInterval(() => void tick(), 3000);
  return () => { stop = true; clearInterval(timer); };
}, [reqState, reqId, onPaired]);
```

渲染：`idle` 态在 token 表单下加分隔线 + 设备名输入 + 「请求接入」按钮 + `msg` 提示；`waiting` 态显示「等待桌面批准…（5 分钟内有效）」+ 4 位码输入 + 提交按钮 + `msg`。（全部沿用 PairPage 既有的浅色/dark 双态类名风格。）

- [ ] **Step 4: 跑测试 + 门禁**

Run: `pnpm vitest run tests/mobile/ && pnpm check && pnpm build:mobile`
Expected: 全过（既有 App/PairPage 用例零回归）。

- [ ] **Step 5: 提交**

```bash
git add src/mobile/api.ts src/mobile/PairPage.tsx tests/mobile/api.test.ts tests/mobile/PairRequest.test.tsx
git commit -m "feat(m4-t2): 移动端请求接入流——设备名+请求/轮询/4位码确认（满员/限试/过期文案）"
```

---

### Task 10: T3 · 电源保活（macOS caffeinate / Windows 执行状态 + 磁盘休眠代设）

**Files:**
- Create: `src-tauri/src/remote/power.rs`
- Modify: `src-tauri/src/remote/mod.rs`（`pub mod power;` + KEY + 启停接线 + 恢复）
- Modify: `src-tauri/Cargo.toml`（windows features 追加 `Win32_System_Power`）
- Modify: `src/components/settings/RemoteSection.tsx` + i18n（保活开关 + 边界文案）

**Interfaces:**
- Produces: `power::acquire()/release()/restore_on_launch()`；纯核 `power::should_acquire(setting: Option<String>) -> bool`（默认开）、`power::PowerCore<O: PowerOps>`（注入式状态机，可测）；KEY_KEEPALIVE="remote.keepalive"。
- Consumes: `start_server`/`stop_server` 生命周期。

- [ ] **Step 1: 写失败测试**

`power.rs` 一体（先写测试后补实现同文件）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeOps {
        started: RefCell<Vec<&'static str>>,
        disk_read: Option<u32>,
        disk_writes: RefCell<Vec<Option<u32>>>, // None=清除（恢复后）
        persisted: RefCell<Vec<Option<u32>>>,
    }
    impl PowerOps for FakeOps {
        fn exec_start(&mut self) { self.started.borrow_mut().push("start"); }
        fn exec_stop(&mut self) { self.started.borrow_mut().push("stop"); }
        fn disk_read(&mut self) -> Option<u32> { self.disk_read }
        fn disk_set(&mut self, v: Option<u32>) { self.disk_writes.borrow_mut().push(v); }
        fn persist_saved(&mut self, v: Option<u32>) { self.persisted.borrow_mut().push(v); }
    }

    #[test]
    fn acquire_then_release_restores_disk() {
        let mut core = PowerCore::new(FakeOps { disk_read: Some(600), ..Default::default() });
        assert!(core.acquire());   // start + 磁盘 600→0 + 持久化 600
        assert!(core.acquire());   // 幂等：重复 acquire 不再 start
        core.release();            // stop + 磁盘恢复 600 + 持久化清除
        core.release();            // 幂等
        let ops = core.into_ops();
        assert_eq!(&*ops.started.borrow(), &["start", "stop"]);
        assert_eq!(&*ops.disk_writes.borrow(), &[Some(0), Some(600)]);
        assert_eq!(&*ops.persisted.borrow(), &[Some(600), None]);
    }

    #[test]
    fn keepalive_default_on_and_opt_out() {
        assert!(should_acquire(None));
        assert!(should_acquire(Some("true".into())));
        assert!(!should_acquire(Some("false".into())));
        assert!(should_acquire(Some("乱串".into()))); // 乱串回落默认开（fail-safe）
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test power` → 编译失败。

- [ ] **Step 3: 实现**

```rust
// 电源保活（M4 T3，宪法 D13）：远程开启期间阻止系统空闲睡眠与磁盘休眠（屏幕可熄）。
// macOS = caffeinate -ims 子进程（随 release kill）；Windows = 常驻线程
// SetThreadExecutionState(ES_CONTINUOUS|ES_SYSTEM_REQUIRED) + 磁盘休眠代设 0/还原
// （PowerReadACValueIndex/PowerWriteACValueIndex，代设前持久化原值——崩溃后下次启动还原）。
// 可测核 PowerCore 注入式状态机（同 PairingClock 先例）；OS 调用集中在 RealOps。

/// 保活开关键（默认开）
pub const KEY_KEEPALIVE: &str = "remote.keepalive";
/// Windows 磁盘休眠原值（崩溃恢复用；有值 = 上次未正常还原）
const KEY_WIN_DISK_SAVED: &str = "remote.win_disk_idle_saved";

/// 开关解析（纯函数）：None/乱串 → true（fail-safe：保活默认开，spec T3）
pub fn should_acquire(setting: Option<String>) -> bool {
    setting.as_deref().map(|v| v != "false").unwrap_or(true)
}

/// OS 操作注入缝（单测 Fake；生产 Real 见下）
pub trait PowerOps {
    fn exec_start(&mut self);
    fn exec_stop(&mut self);
    /// 读当前（AC）磁盘休眠秒数；平台不支持/读失败 → None（跳过磁盘代设）
    fn disk_read(&mut self) -> Option<u32>;
    /// 写磁盘休眠秒数；None = 恢复语义由实现自行处理（Real: 用持久化原值）
    fn disk_set(&mut self, v: Option<u32>);
    /// 持久化原值（None = 清除——正常还原后清键）
    fn persist_saved(&mut self, v: Option<u32>);
}

/// 状态机核（可测）：acquire 幂等 + 磁盘代设/还原 + 原值持久化
pub struct PowerCore<O: PowerOps> {
    ops: O,
    held: bool,
    saved_disk: Option<u32>,
}
impl<O: PowerOps> PowerCore<O> {
    pub fn new(ops: O) -> Self { Self { ops, held: false, saved_disk: None } }
    pub fn acquire(&mut self) -> bool {
        if self.held { return false; }
        self.ops.exec_start();
        if let Some(cur) = self.ops.disk_read() {
            if cur != 0 { // 已是「永不」则不动（还原也跳过）
                self.saved_disk = Some(cur);
                self.ops.persist_saved(Some(cur));
                self.ops.disk_set(Some(0));
            }
        }
        self.held = true;
        true
    }
    pub fn release(&mut self) {
        if !self.held { return; }
        self.ops.exec_stop();
        if let Some(v) = self.saved_disk.take() {
            self.ops.disk_set(Some(v));
            self.ops.persist_saved(None);
        }
        self.held = false;
    }
    pub fn into_ops(self) -> O { self.ops }
}
```

生产实现（平台门控）+ 全局句柄：

```rust
struct RealOps;
#[cfg(target_os = "macos")]
impl PowerOps for RealOps {
    fn exec_start(&mut self) { /* caffeinate -ims 存到全局 CHILD */ real_start_mac(); }
    fn exec_stop(&mut self) { real_stop_mac(); }
    fn disk_read(&mut self) -> Option<u32> { None } // macOS 磁盘休眠由 caffeinate -m 覆盖
    fn disk_set(&mut self, _: Option<u32>) {}
    fn persist_saved(&mut self, _: Option<u32>) {}
}
// （Windows RealOps：exec_start=起常驻线程 SetThreadExecutionState；disk_read/set=
//   Power{Read,Write}ACValueIndex；persist_saved=KV 读写。linux：exec_* no-op log。）

static POWER: Lazy<std::sync::Mutex<PowerCore<RealOps>>> = Lazy::new(|| {
    std::sync::Mutex::new(PowerCore::new(RealOps))
});

/// 远程开启时获取（开关默认开；toggle 侧调用）
pub fn acquire() {
    if !should_acquire(crate::database::dao::settings::get_setting(KEY_KEEPALIVE)) { return; }
    let _ = &*POWER; // Windows 依赖 Lazy 初始化副作用（无——保守保活）
    POWER.lock().unwrap().acquire();
}
pub fn release() { POWER.lock().unwrap().release(); }

/// 崩溃恢复：启动时若 KV 有未还原的磁盘原值 → 写回并清键（lib.rs setup 调）
pub fn restore_on_launch() {
    let saved = crate::database::dao::settings::get_setting(KEY_WIN_DISK_SAVED)
        .and_then(|v| v.parse::<u32>().ok());
    if let Some(v) = saved {
        let mut ops = RealOps;
        ops.disk_set(Some(v));
        ops.persist_saved(None);
        log::info!("电源保活：恢复崩溃前的磁盘休眠设置 {v}s");
    }
}
```

macOS 子进程管理：

```rust
#[cfg(target_os = "macos")]
static CAFFEINATE: Lazy<std::sync::Mutex<Option<std::process::Child>>> = Lazy::new(|| std::sync::Mutex::new(None));
#[cfg(target_os = "macos")]
fn real_start_mac() {
    let mut g = CAFFEINATE.lock().unwrap();
    if g.is_some() { return; }
    match std::process::Command::new("caffeinate").args(["-ims"]).spawn() {
        Ok(c) => *g = Some(c),
        Err(e) => log::warn!("caffeinate 启动失败（保活降级）: {e}"),
    }
}
#[cfg(target_os = "macos")]
fn real_stop_mac() {
    if let Some(mut c) = CAFFEINATE.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}
```

Windows RealOps（windows crate 0.57；签名以 `cargo check` 报错为准微调——GUID 常量与流程如下）：

```rust
#[cfg(windows)]
mod win {
    use std::sync::{Lazy, Mutex};
    use windows::Win32::System::Power::{
        PowerGetActiveScheme, PowerReadACValueIndex, PowerWriteACValueIndex,
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };
    // SUB_DISK / DISKIDLE 官方 GUID（磁盘子组 / 磁盘空闲超时）
    const SUB_DISK: windows::core::GUID = windows::core::GUID::from_u128(0x0012ee47_9041_4b5d_9b77_535fba8b1442);
    const DISKIDLE: windows::core::GUID = windows::core::GUID::from_u128(0x6738e2c4_e8a5_4a42_b16a_e040e769756e);

    static THREAD_STOP: Lazy<Mutex<Option<std::sync::mpsc::Sender<()>>>> = Lazy::new(|| Mutex::new(None));

    pub fn exec_start() {
        let mut g = THREAD_STOP.lock().unwrap();
        if g.is_some() { return; }
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            // ES_CONTINUOUS 断言绑定线程：常驻线程持有，收到 stop 才清除并退出
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED); }
            let _ = rx.recv();
            unsafe { SetThreadExecutionState(ES_CONTINUOUS); }
        });
        *g = Some(tx);
    }
    pub fn exec_stop() {
        if let Some(tx) = THREAD_STOP.lock().unwrap().take() { drop(tx); } // recv Err → 线程清断言退出
    }
    pub fn disk_read() -> Option<u32> {
        unsafe {
            let mut scheme = std::ptr::null_mut();
            if PowerGetActiveScheme(None, &mut scheme).is_err() { return None; }
            let mut v = 0u32;
            let ok = PowerReadACValueIndex(None, *scheme, &SUB_DISK, &DISKIDLE, &mut v).is_ok();
            // scheme 指针由 API 分配，需 LocalFree 释放（防泄漏）
            if !scheme.is_null() {
                let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(scheme as _)));
            }
            ok.then_some(v)
        }
    }
    pub fn disk_set(v: u32) {
        unsafe {
            let mut scheme = std::ptr::null_mut();
            if PowerGetActiveScheme(None, &mut scheme).is_err() { return; }
            let _ = PowerWriteACValueIndex(None, *scheme, &SUB_DISK, &DISKIDLE, v);
            let _ = windows::Win32::System::Power::PowerSetActiveScheme(None, *scheme);
            if !scheme.is_null() {
                let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(scheme as _)));
            }
        }
    }
}
```

（`PowerGetActiveScheme` 第一个参数 windows 0.57 中为 `rootpowerkey: Option<HKEY>` 形态、`PowerReadACValueIndex` 参数为 `Option<HKEY>, GUID, GUID, GUID?...`——以编译器报错为准调整 None/指针形态；流程与 GUID 不变。**Cargo.toml**：windows 依赖 features 追加 `"Win32_System_Power"`。）

`mod.rs` 接线：
1. `start_server()` 内 spawn 成功后（`start_server_core` 返回 Ok(true) 分支）加 `power::acquire();`
2. `stop_server()` 末尾加 `power::release();`
3. `restore_on_launch()` 函数体首行加 `power::restore_on_launch();`
4. 常量区已在 power.rs 定义 KEY——无需重复。

设置页（`RemoteSection.tsx`）开关区块（远程开关行下）：

```tsx
{/* 电源保活（M4 T3，默认开）：远程开启期间阻止休眠（屏幕可熄） */}
<div className="flex items-center justify-between py-2.5">
  <div className="flex-1">
    <label className="text-sm font-medium">{t("settings.remote.keepalive")}</label>
    <p className="text-muted-foreground mt-0.5 text-xs">{t("settings.remote.keepaliveHint")}</p>
  </div>
  <Switch checked={keepalive} disabled={busy || !status}
    onCheckedChange={(v) => void changeKeepalive(v)} />
</div>
```

（`keepalive` 态 = `getSetting("remote.keepalive") !== "false"`；`changeKeepalive` = `setSetting("remote.keepalive", v ? "true" : "false")` + refresh。）i18n：`keepalive` 电源保活/Keep awake、`keepaliveHint`「远程开启期间阻止系统与磁盘休眠（屏幕可熄）。macOS 合盖仍会休眠；电池模式下 macOS 需接电源才完整生效、Windows 临界电量会强制睡眠」/英文对照。

- [ ] **Step 4: 跑测试 + 门禁**

Run: `cd src-tauri && cargo test power && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt && cd .. && pnpm check`
Expected: 全绿（Windows 分支代码在 mac 编译门下至少语法过——CI windows job 兜底真编译）。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/remote/power.rs src-tauri/src/remote/mod.rs src-tauri/Cargo.toml src/components/settings/RemoteSection.tsx src/i18n/locales/zh.json src/i18n/locales/en.json
git commit -m "feat(m4-t3): 电源保活——caffeinate/执行状态+磁盘休眠代设还原（注入式状态机+崩溃恢复），默认开+边界文案"
```

---

### Task 11: T4 · 托盘远程入口

**Files:**
- Modify: `src-tauri/src/remote/mod.rs`（`tray_snapshot()` 纯装配）
- Modify: `src-tauri/src/plugins/system_tray.rs`（两处菜单重建路径加远程项 + 事件处理）
- Modify: `src-tauri/src/lib.rs`（update_tray_menu 命令签名）
- Modify: `src/pages/home.tsx`、`src/components/common/language-toggle.tsx`（调用点传新参）
- Modify: `src/hooks/useRemoteEvents.ts`（remote-changed → 重建托盘）
- Modify: `src/tauri-mock.ts`
- Test: `src-tauri` mod tests（tray_snapshot 纯核）

**Interfaces:**
- Produces: `mod::tray_display() -> (bool, String)`（enabled + 对外地址 = 隧道优先）；托盘菜单项 id `"remote"`（CheckMenuItem）/`"remote-addr"`（展示项）；命令 `update_tray_menu(show_text, quit_text, pet_text, remote_text, remote_off_text)`。
- Consumes: Task 3 emit、Task 5 tunnel snapshot、Task 8 useRemoteEvents。

- [ ] **Step 1: 写失败测试（托盘展示纯核）**

`mod.rs` tests：

```rust
    #[test]
    fn tray_display_prefers_tunnel_url() {
        // 纯核：隧道开 → (true, 隧道地址)；关 → (false, 绑定口径地址)
        assert_eq!(tray_display_from(true, Some("https://mam.example.asia".into()), "http://192.168.1.5:9420/m".into()), (true, "https://mam.example.asia".into()));
        assert_eq!(tray_display_from(true, None, "http://192.168.1.5:9420/m".into()), (true, "http://192.168.1.5:9420/m".into()));
        assert_eq!(tray_display_from(false, None, "http://127.0.0.1:9420/m".into()), (false, String::new()));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test tray_display` → 编译失败。

- [ ] **Step 3: 实现**

`mod.rs`：

```rust
/// 托盘展示纯核（M4 T4）：enabled=false 不给地址（无服务可连）；
/// 隧道开地址优先（T1a 同口径）
pub fn tray_display_from(
    enabled: bool,
    tunnel_url: Option<String>,
    bind_url: String,
) -> (bool, String) {
    if !enabled { return (false, String::new()); }
    (true, tunnel_url.unwrap_or(bind_url))
}

/// 托盘快照（system_tray 消费；与 remote_status 同源不复制聚合——直调各单点）
pub fn tray_display() -> (bool, String) {
    let (bind, port) = bind_and_port().unwrap_or(("127.0.0.1".to_string(), DEFAULT_PORT));
    let enabled = status_enabled(
        crate::database::dao::settings::get_setting(KEY_ENABLED).map(|v| v == "true").unwrap_or(false),
        handle_is_live(&SERVER_HANDLE.lock().unwrap()),
    );
    let tun = tunnel::snapshot();
    tray_display_from(
        enabled,
        tun.url.filter(|_| tun.error.is_none()),
        display_url_for(&bind, port, local_lan_ips()),
    )
}
```

`system_tray.rs`：
1. `update_tray_menu` 与 `init` 的菜单构建统一走新内部函数（两处 + presets 共三处消费）：

```rust
/// 远程区菜单项（M4 T4）：开关（Check）+ 地址展示（disabled，点击经事件走前端剪贴板）
fn remote_items(app: &AppHandle, remote_on_text: &str) -> Result<Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>>, String> {
    let (enabled, addr) = crate::remote::tray_display();
    let toggle = tauri::menu::CheckMenuItem::with_id(
        app, "remote", remote_on_text, true, enabled, None::<&str>,
    ).map_err(|e| e.to_string())?;
    let addr_item = tauri::menu::MenuItem::with_id(
        app, "remote-addr",
        &if addr.is_empty() { "—".to_string() } else { addr },
        true, None::<&str>,
    ).map_err(|e| e.to_string())?;
    let sep = tauri::menu::PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    Ok(vec![Box::new(sep), Box::new(toggle), Box::new(addr_item)])
}
```

（`update_tray_menu` 把 `remote_items` 追加到既有 show/pet/quit 之间——quit 之前；`update_tray_with_presets` 同样追加；`init` 默认菜单亦然。签名 `update_tray_menu(app, show_text, quit_text, pet_text, remote_on_text: String)`。）
2. `on_menu_event` match 追加：

```rust
                    "remote" => {
                        // 托盘开关：与设置页同一命令链路（remote_toggle 内含 TLS 门回滚）
                        let (_, addr) = crate::remote::tray_display();
                        let enabled = !addr.is_empty() || {
                            // 地址空可能=关 or 无服务：以 DB+句柄口径判当前态
                            let (e, _) = crate::remote::tray_display();
                            e
                        };
                        let next = !enabled;
                        match crate::remote::remote_toggle(next) {
                            Ok(()) => {
                                // 菜单勾选态经前端回环刷新（remote-changed 监听重建）
                                crate::remote::events::emit_ui("remote-changed", serde_json::json!({"enabled": next}));
                            }
                            Err(e) => {
                                crate::remote::events::emit_ui("remote-toggle-failed", serde_json::json!({"error": e}));
                            }
                        }
                    }
                    "remote-addr" => {
                        let (_, addr) = crate::remote::tray_display();
                        if !addr.is_empty() {
                            crate::remote::events::emit_ui("remote-copy-addr", serde_json::json!({"url": addr}));
                        }
                    }
```

（**简化**：`remote_toggle(next)` 的 next 判定直接用 `tray_display().0`：`let next = !crate::remote::tray_display().0;`——删除上面多余的 addr 推导。）

`lib.rs` 命令签名：

```rust
#[tauri::command]
fn update_tray_menu(
    app: tauri::AppHandle,
    show_text: String,
    quit_text: String,
    pet_text: String,
    remote_on_text: String,
) -> Result<(), String> {
    plugins::system_tray::update_tray_menu(&app, &show_text, &quit_text, &pet_text, &remote_on_text)
}
```

前端两调用点补参 `remoteOnText: t("tray.remote", "远程接入")`（home.tsx / language-toggle.tsx——具体 i18n 键沿用两文件现有取键风格，加 `tray.remote` zh=远程接入 en=Remote Access）。

`useRemoteEvents.ts` 追加监听：

```ts
      unlisten.push(await listen("remote-changed", async () => {
        // 状态变了 → 重建托盘菜单（勾选态/地址刷新；标签走前端本地化）
        try {
          await invoke("update_tray_menu", {
            showText: i18n.t("tray.show"), quitText: i18n.t("tray.quit"),
            petText: i18n.t("tray.pet"), remoteOnText: i18n.t("tray.remote"),
          });
        } catch { /* 托盘刷新尽力而为 */ }
      }));
```

（`tray.show/quit/pet` 键若不存在则按两调用点现有键名复用——执行时以 home.tsx 现传值为准对齐，不新造重复键。）

`tauri-mock.ts`：`case "update_tray_menu":` 兼容新旧参（多余参数忽略即可，无返回）。

- [ ] **Step 4: 跑测试 + 门禁**

Run: `cd src-tauri && cargo test tray_display && cargo clippy --all-targets -- -D warnings && cargo fmt && cd .. && pnpm check && pnpm test`
Expected: 全绿。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/remote/mod.rs src-tauri/src/plugins/system_tray.rs src-tauri/src/lib.rs src/pages/home.tsx src/components/common/language-toggle.tsx src/hooks/useRemoteEvents.ts src/tauri-mock.ts src/i18n/locales/zh.json src/i18n/locales/en.json
git commit -m "feat(m4-t4): 托盘远程入口——开关勾选项+地址展示复制（三菜单重建路径统一+remote-changed 回环刷新）"
```

---

### Task 12: 收尾 · 全量门禁 + M4 验收记录

**Files:**
- Create: `docs/release-notes/m4-acceptance.md`

**Interfaces:** 无代码——验收文档 + 门禁终跑。

- [ ] **Step 1: 全量门禁（顺序执行，任何一步失败停下修复）**

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd .. && pnpm build:mobile && pnpm check && pnpm test
```

Expected: 全绿；记录各套数字（Rust 用例数 / 前端用例数 / 双端构建产物）写入验收记录。

- [ ] **Step 2: 写验收记录**

`docs/release-notes/m4-acceptance.md`（对照 `m3-acceptance.md` 结构）：分支/基线（`feat/m4-external-fullchain`，基于 M3 合并后 main）、逐任务交付表（T0-T4 → commit hash）、自动化门禁数字、**手动验收清单**（spec §9 八项外网真机 + 局域网补充项逐条列出待用户勾选）、已知限制登记（named 隧道 url 解析失败时设置页显示「以 Cloudflare 面板为准」、审批队列重启即清、托盘通知点击直达设置页为观察项）。

- [ ] **Step 3: 更新 spec 附录进度表**

`docs/superpowers/specs/2026-09-12-remote-access-level1-design.md` 附录 B（B2/B4/B5 ✅ 待实测/🔄）与进度基线行——按「代码完成待用户验收」口径更新（M3 收官后本表同样需补 M3 ✅，一并处理）。

- [ ] **Step 4: 提交**

```bash
git add docs/release-notes/m4-acceptance.md docs/superpowers/specs/2026-09-12-remote-access-level1-design.md
git commit -m "docs(m4): M4 验收记录（自动化门禁数字+真机手动清单）+ spec 附录进度表更新"
```

- [ ] **Step 5: 收尾**

分支保持 `feat/m4-external-fullchain`；提 PR（标题 `M4 外网全链路：T0 清账 + T1 隧道 + T2 审批配对 + T3 保活 + T4 托盘`），PR 正文粘贴手动验收清单，**合并决定留给用户**。

---

## 自审记录（writing-plans Self-Review）

1. **Spec 覆盖**：T0a→Task 1；T0b→Task 2；T1a→Task 6；T1b/T1c→Task 5；T1d→Task 4；T1e→已内建（spec 声明无交付，验收 2 附带）；T1f→Task 6 Tailscale 文案；T2a→Task 7/9；T2b→Task 7/8/9；T2c→Task 7（pair 上限门+approve+confirm）；T2d→Task 7/8（在线口径 is_online）；T2e→Task 3 audit 贯穿；T3→Task 10；T4→Task 11；验收 8 项→Task 12 手动清单。**无缺口**。
2. **占位符扫描**：Task 3 Step 2 初稿的 `call_once/unreachable!` 占位已在同步内修正为 OnceLock 方案；Windows Power API 标注「以 cargo check 报错为准微调签名」（流程/GUID 已定，属编译适配非占位）；i18n Task 8 标注「以实现时实际文案成对补齐」但已列全键名清单。其余无 TBD/TODO。
3. **类型一致性**：`device_cookie`（Task 7 定义，api::pair 同步改用）；`max_devices()`（Task 7 mod.rs，api.rs confirm 消费同签名）；`tray_display_from`（Task 11 定义/测试同签名）；`address_entries_with_tunnel`/`pair_url_with_tunnel`（Task 5 定义，测试同名）；前端 `RemoteAddressEntry.kind`（Task 6 定义，Task 8 复用）；事件名 `remote-pair-request/remote-changed/remote-tunnel-address/remote-tunnel-error/remote-copy-addr/remote-toggle-failed/remote-roster-changed` 后端 emit 与前端 listen 逐对核对一致。
