#![cfg(windows)]
//! Windows ConPTY 注入通道（M9R 执行层重写，M6R–M9R 批次 Task 3）：`AttachConsole`
//! → 打开 `CONIN$` → `WriteConsoleInput` 写键事件。两处 M6 探测实证仍有效（落地照做）：
//! 1. AttachConsole 后 `GetStdHandle` 返回旧控制台陈旧句柄（stdin 为管道时必败）
//!    ——必须以 `CONIN$` 显式打开；
//! 2. `INPUT_RECORD` 必须拍平为 blittable 布局（嵌套结构体在 FFI 封送报
//!    0x8007007A）——本文件自定 20 字节平铺结构指针直传，与 ConIn.ps1 的
//!    C# 平铺完全同构。
//!
//! ## M9R 执行纪律（本文件落实项，证据 `research/refs/phase2-消息注入/`）
//! - **P1-2 进程级互斥**：控制台附加态进程全局唯一，[`CONSOLE_OP`] 串行化
//!   「定位 → 附加 → 写入 → 复位」全程单临界区——公共入口 [`inject_text_spec`] /
//!   [`inject_key_spec`] 各取一次锁；[`resolve_target`] 与 [`inject_via`] 为
//!   **无锁内部版**（调用方已持锁），禁止嵌套取锁（std Mutex 不可重入，嵌套即死锁）；
//! - **P1-1 RAII 复位**：[`AttachGuard`] Drop → `FreeConsole` 无条件复位，任何
//!   错误/panic 早退路径都兜底——滞留附加列表的进程在宿主终端窗口关闭时会收到
//!   CTRL_CLOSE_EVENT，本应用不装 SetConsoleCtrlHandler，默认行为是**被系统终止**
//!   （用户关一个收过消息的终端 = MAM 整个应用跟着退出，故复位不可省，计划明文）；
//! - **自适应节流（§8.1）**：按族规格（`super::families`，M6R 探测定案表）分流——
//!   快消费者 80 字符/块（160 事件单次写入）、块间 50ms；慢消费者/长文切背压，
//!   每块后排空目标输入缓冲占用至 ≤ DRAIN_TO(40) 再续写（15ms 轮询）；
//! - **P2-1 真总预算**：正文 / 提交回车 / 122 重试 / 背压轮询共享一个 deadline
//!   （`families::inject_budget_ms`，背压按斜率放宽），每轮循环顶部检查，耗尽即
//!   中文报错（可重试语义）；
//! - **部分写续写（M6R 纪律）**：`WriteConsoleInput` 实写 < 分片长 → 从偏移续写
//!   （续写不睡眠）；122（ERROR_INSUFFICIENT_BUFFER）现语义 = 目标读武装期缓冲
//!   暂满、可重试（M6「消费墙首读预算 13~43 字符」叙事已被 M6R 推翻）→ 100ms
//!   重试同块；错误码仅在 Err 时读取（F7：ok=true 时错误码为噪声）；
//! - **P2-2 键域校验**：域外键在取锁/附加之前快速失败并报错，**不回退文本+回车**；
//! - **Task 4 占用监控 + 冻结自愈（§8.1「背压与异常」行 / M6R F1）**：[`paced_write`]
//!   在块写入后 / 背压轮询处 / 122·Ok(0) 重试睡眠后采样目标输入缓冲占用——
//!   `pending>0` 连续 ≥ `families::OCC_ABNORMAL_MS`(5s) 且无净下降判冻结（宿主
//!   选择模式：写入成功 TUI 读不到）→ [`try_unfreeze`] 向宿主窗口 PostMessage
//!   WM_KEYDOWN/WM_KEYUP VK_ESCAPE 解冻（成功置 [`InjectStats::unfroze`] 继续写；
//!   失败报中文错误——WT 宿主恢复未验证由该文案覆盖，§8.3 实机复验项）；
//! - **Task 4 屏读层（CONOUT$）**：[`read_input_tail`] 读输入行尾部（直发滞留
//!   回查与 Task 5 插队草稿确认用），与注入共用 [`CONSOLE_OP`] 单临界区；
//! - **实机验证归属**：分块/预算/背压/续写/占用监控/冻结自愈/屏读的实机行为验证
//!   归 Task 12 `#[ignore]`（本任务门禁只锁编译 + 单测 + 既有 FFI 契约测试）。

use std::thread::sleep;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use windows::core::Error as WinError;
use windows::core::HRESULT;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, GENERIC_READ, GENERIC_WRITE, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    AttachConsole, FreeConsole, GetConsoleScreenBufferInfo, GetConsoleWindow,
    GetNumberOfConsoleInputEvents, ReadConsoleOutputCharacterW, WriteConsoleInputW,
    CONSOLE_SCREEN_BUFFER_INFO, COORD, INPUT_RECORD, KEY_EVENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN, WM_KEYUP};

use super::engine::{
    control_records, enter_records, text_records, vk_arrow_records, vt_seq_records, KeyLayout,
    KeyRecordSpec,
};
use super::families::{self, FamilySpec, TuiFamily};

/// 122 重试间隔（§8.1：122 走 100ms 重试同块，受 deadline 约束）
const RETRY_GAP: Duration = Duration::from_millis(100);
/// 背压轮询间隔（§8.1：排空目标输入缓冲的 15ms 轮询步距，受 deadline 约束）
const DRAIN_POLL_GAP: Duration = Duration::from_millis(15);
/// 冻结自愈等待上限（§8.1：解冻发帖后等 drain 的最长时间；实际取
/// `min(此值, 调用方 deadline 剩余)`，deadline 耗尽走既有超时错误路径）
const UNFREEZE_WAIT_MS: u64 = 2_000;
/// `WM_KEYDOWN` lparam（本地常量，标准键消息布局）：repeat=1、scancode=0x01
/// （ESC 扫描码）、extended/context/previous/transition = 0。
const ESC_LPARAM_DOWN: isize = 0x0001_0001;
/// `WM_KEYUP` lparam（本地常量）：repeat=1、scancode=0x01、previous=1 +
/// transition=1（抬起位，0xC0 高段）。
const ESC_LPARAM_UP: isize = 0xC001_0001;

/// P1-2：进程级注入互斥。控制台「附加态」是进程全局唯一的资源，任何并发注入
/// （多会话同时 flush）都会互踩 attach/detach；此处串行化全部
/// 「定位 → 附加 → 写入 → 复位」临界区。锁纪律：公共入口各取一次锁；
/// `resolve_target` / `inject_via` 为无锁内部版，严禁嵌套取锁。
///
/// 最坏持锁时长 ≈ `families::inject_budget_ms`（背压按 45ms/字符斜率放宽，
/// 万字符最高 ≈460s）；等待方的 deadline 在**取锁之后**才起算——排队等待不计入
/// 自身预算，只会排队而不会误报超时。Task 4 落地：`read_input_tail`（屏读层）
/// 与注入共享此临界区（附加态的读侧同样互斥，杜绝读屏与写入互踩附加态/
/// 交错改读同一输入行）；`try_unfreeze` 只在锁内被调用、自身不取锁。
static CONSOLE_OP: Lazy<std::sync::Mutex<()>> = Lazy::new(|| std::sync::Mutex::new(()));

/// P1-1：附加态 RAII 守卫——构造前调用方已 `AttachConsole` 成功；Drop 时无条件
/// `FreeConsole` 复位。任何错误/panic 早退路径都经过 Drop（旧实现手工收尾，
/// `?` 早退/panic 路径会漏出滞留附加态 → CTRL_CLOSE_EVENT 连带终止风险）。
struct AttachGuard;
impl Drop for AttachGuard {
    fn drop(&mut self) {
        // SAFETY: FFI 调用；FreeConsole 幂等（未附加时也安全，M6 探测实证）
        unsafe {
            let _ = FreeConsole();
        }
    }
}

/// 注入结果统计（跨任务接口契约，Task 4/5 消费）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InjectStats {
    /// 成功送达的正文字符数（`text.chars().count()`；Ok 即全量送达，不含提交回车）
    pub written: usize,
    /// 本次注入是否走背压节流（§8.1 自适应：慢消费者或长文）
    pub backpressure: bool,
    /// 冻结自愈是否发生（Task 4：占用监控判冻 → PostMessage ESC 解冻成功置位，§8.1）
    pub unfroze: bool,
}

/// [`paced_write`] 结果（内部类型，Task 4）：`written` = 实写事件总数（含部分写
/// 续写累计）；`unfroze` = 写入途中是否触发占用监控判冻并成功解冻（PostMessage
/// ESC，§8.1）——text 路径由调用方透传进 [`InjectStats::unfroze`]（key 路径无
/// stats，不透传）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PacedOutcome {
    /// 成功写入的事件总数（含部分写续写累计）
    pub written: usize,
    /// 写入途中是否成功解冻过（占用监控判冻 → [`try_unfreeze`] 成功）
    pub unfroze: bool,
}

/// 平铺 KEY_EVENT `INPUT_RECORD`（blittable 20 字节）：EventType u16 + Reserved u16 +
/// bKeyDown i32(BOOL) + wRepeatCount u16 + wVirtualKeyCode u16 + wVirtualScanCode u16 +
/// UnicodeChar u16 + dwControlKeyState u32。
/// M6 教训：windows-rs 的嵌套 union 形态在 FFI 封送报 0x8007007A，必须平铺直传。
#[repr(C)]
struct FlatKeyRecord {
    event_type: u16,
    /// union 对齐填充位（保留 0）
    _reserved: u16,
    /// BOOL
    key_down: i32,
    repeat_count: u16,
    virtual_key_code: u16,
    virtual_scan_code: u16,
    unicode_char: u16,
    control_key_state: u32,
}
const _: () = assert!(std::mem::size_of::<FlatKeyRecord>() == 20);
const _: () = assert!(
    std::mem::size_of::<FlatKeyRecord>() == std::mem::size_of::<INPUT_RECORD>()
        && std::mem::align_of::<FlatKeyRecord>() == std::mem::align_of::<INPUT_RECORD>()
);

impl From<&KeyRecordSpec> for FlatKeyRecord {
    fn from(spec: &KeyRecordSpec) -> Self {
        Self {
            event_type: KEY_EVENT as u16, // 键事件类型 = 1
            _reserved: 0,
            key_down: i32::from(spec.down),
            repeat_count: 1,
            virtual_key_code: spec.vk,
            // M9R：扫描码随规格下发（VK 形态事件必带，B 族 crossterm 以 VK+scan 为准）
            virtual_scan_code: spec.scan,
            unicode_char: spec.ch,
            control_key_state: 0,
        }
    }
}

/// 真实键位布局（M9R，Windows FFI，供调用点迁移；执行层大重写归 Task 3）。
/// 依据 M6R §8.1 定案：vk = VkKeyScanW(ch) & 0xFF，scan = MapVirtualKeyW(vk)。
pub(crate) struct WinKeyLayout;

#[cfg(windows)]
impl KeyLayout for WinKeyLayout {
    /// 字符 → 虚拟键码：`'\r'` 直返 VK_RETURN（控制字符的 VkKeyScanW 语义不可靠，
    /// 回车 VK 形态三家统一）；非 ASCII → 0（纯字符流，与现役 ConIn.ps1 口径
    /// 一致）；ASCII → `VkKeyScanW`：返回 i16，-1（不可键入，如部分控制字符）
    /// → 0 走字符流，否则低字节为 VK（高字节为 shift/Ctrl/Alt 修饰位，M9R 事件
    /// 不带修饰态，丢弃）。
    fn vk_of(&self, c: char) -> u16 {
        if c == '\r' {
            return 0x0D; // VK_RETURN
        }
        if !c.is_ascii() {
            return 0;
        }
        // SAFETY: FFI 调用，无特殊前置条件
        let ret = unsafe { VkKeyScanW(c as u16) };
        if ret == -1 {
            0
        } else {
            (ret & 0xFF) as u16
        }
    }

    /// 虚拟键码 → 扫描码（`MAPVK_VK_TO_VSC`；无映射返回 0）。
    fn scan_of(&self, vk: u16) -> u16 {
        // SAFETY: FFI 调用，无特殊前置条件
        unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 }
    }
}

/// 附加到目标进程控制台：`FreeConsole`（幂等，未附加时也安全，M6 探测实证）
/// → `AttachConsole`。失败返回错误码（HRESULT 原值），由调用方决定是否回退祖先链。
fn attach(pid: u32) -> Result<(), u32> {
    let _ = unsafe { FreeConsole() };
    if let Err(e) = unsafe { AttachConsole(pid) } {
        log::warn!("AttachConsole(pid={pid}) 失败：0x{:08X}", e.code().0 as u32);
        return Err(e.code().0 as u32);
    }
    Ok(())
}

/// 打开目标控制台输入缓冲（`CONIN$`）。M6 修正：不能用 `GetStdHandle`
/// （附加后返回旧控制台陈旧句柄，stdin 为管道时必败）。
/// 需要读+写权（WriteConsoleInput 要求）；共享读写以兼容目标进程自身访问。
///
/// # SAFETY
/// FFI 调用：成功句柄由调用方负责 `CloseHandle` 收尾。
unsafe fn open_conin() -> Result<HANDLE, String> {
    CreateFileW(
        windows::core::w!("CONIN$"),
        GENERIC_READ.0 | GENERIC_WRITE.0,
        FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0),
        None,
        OPEN_EXISTING,
        FILE_FLAGS_AND_ATTRIBUTES(0),
        HANDLE(0),
    )
    .map_err(|e| format!("打开 CONIN$ 失败（0x{:08X}）", e.code().0 as u32))
}

/// 打开目标控制台屏幕缓冲（`CONOUT$`，读+写权，Task 4 屏读层）。读权供
/// `ReadConsoleOutputCharacterW`；共享读写以兼容目标进程自身访问。
///
/// # SAFETY
/// FFI 调用：成功句柄由调用方负责 `CloseHandle` 收尾；须处于目标控制台附加态。
#[allow(dead_code)] // Task 5 确认层消费（插队草稿确认/直发滞留回查）；本任务先落契约
unsafe fn open_conout() -> Result<HANDLE, String> {
    CreateFileW(
        windows::core::w!("CONOUT$"),
        GENERIC_READ.0 | GENERIC_WRITE.0,
        FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0),
        None,
        OPEN_EXISTING,
        FILE_FLAGS_AND_ATTRIBUTES(0),
        HANDLE(0),
    )
    .map_err(|e| format!("打开 CONOUT$ 失败（0x{:08X}）", e.code().0 as u32))
}

/// 单片写入：平铺结构切片按 `INPUT_RECORD` 同构布局指针直传（上方编译期断言锁定
/// 尺寸/对齐一致；windows-rs `WriteConsoleInputW` 内部对切片指针仅 transmute 直通，
/// 无嵌套封马 → 不踩 0x8007007A）。返回 windows Error 以便按错误码分流
/// （122 = 目标读武装期缓冲暂满，可重试）。**返回实写事件数**（M6R 纪律：
/// 实写 < 分片长时调用方从偏移续写）。
///
/// # SAFETY
/// FFI 调用：`handle` 须为有效 CONIN$ 句柄；切片生命周期覆盖调用全程。
unsafe fn write_chunk(handle: HANDLE, chunk: &[FlatKeyRecord]) -> Result<u32, WinError> {
    let records = std::slice::from_raw_parts(chunk.as_ptr() as *const INPUT_RECORD, chunk.len());
    let mut written = 0u32;
    WriteConsoleInputW(handle, records, &mut written).map(|_| written)
}

/// 122 判定：ERROR_INSUFFICIENT_BUFFER(122)（HRESULT 形态 0x8007007A）。
/// 仅在 `write_chunk` 返回 Err 时读错误码（M6R F7：ok=true 时错误码为噪声）。
fn is_insufficient_buffer(e: &WinError) -> bool {
    e.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0)
}

/// 查询目标输入缓冲占用（未消费事件数）。背压排空（Task 3）与占用监控判冻
/// （Task 4）共用同一查询；两者语义分离：`families::DRAIN_TO` 是背压回落阈值
/// （写停到占用 ≤40 续写，量「落到多少续写」），`families::OCC_ABNORMAL_MS` 是
/// 判冻窗口时长（pending>0 连续 ≥5s 且无净下降才判异常，量「停多久」），互不复用。
fn query_pending(handle: HANDLE) -> Result<u32, String> {
    let mut pending = 0u32;
    // SAFETY: FFI 调用；handle 为本调用链刚打开的有效 CONIN$ 句柄
    unsafe { GetNumberOfConsoleInputEvents(handle, &mut pending) }.map_err(|e| {
        format!(
            "GetNumberOfConsoleInputEvents 失败（0x{:08X}）",
            e.code().0 as u32
        )
    })?;
    Ok(pending)
}

/// 占用监控判冻窗口（Task 4，§8.1「背压与异常」行）：**判冻条件 = pending>0
/// 连续 ≥ `families::OCC_ABNORMAL_MS`(5s) 且无净下降**——窗口起点记 baseline，
/// `pending < baseline` 视为有消费进展 → 重置窗口（慢消费者合法 drain 每轮下降
/// 即重置，不会误报；冻结/选择模式 pending 不降才累计到阈值触发）；`pending == 0`
/// → 重置窗口。与 `families::DRAIN_TO`（Task 3 既有背压回落阈值）语义分离，见
/// [`query_pending`] 注。
struct OccWatch {
    /// 本判冻窗口起点占用（窗口打开时的 pending 快照，净下降判定基准）
    baseline: u32,
    /// pending>0 连续起点（None = 窗口未开/已重置）
    since: Option<Instant>,
    /// 本轮写入是否成功解冻过（透传 [`PacedOutcome::unfroze`]）
    unfroze: bool,
}

impl OccWatch {
    fn new() -> Self {
        Self {
            baseline: 0,
            since: None,
            unfroze: false,
        }
    }

    /// 用既有查询结果推进判冻窗口（[`drain_to`] 轮询处复用同一 pending，
    /// 不重复查询）。判冻触发 → [`try_unfreeze`] 自愈：成功置 `unfroze` 并重开
    /// 窗口继续写；失败报中文冻结错误（§8.3：WT 宿主下冻结恢复未验证——实机
    /// 复验项，该文案覆盖此限制并引导人工介入）。
    fn observe(&mut self, pending: u32, handle: HANDLE, deadline: Instant) -> Result<(), String> {
        if pending == 0 {
            self.since = None; // 无积压 → 重置窗口
            return Ok(());
        }
        match self.since {
            None => {
                self.baseline = pending;
                self.since = Some(Instant::now());
            }
            Some(t0) => {
                if pending < self.baseline {
                    // 净下降 = 有消费进展（慢消费者合法 drain）→ 重置窗口
                    self.baseline = pending;
                    self.since = Some(Instant::now());
                } else if t0.elapsed().as_millis() as u64 >= families::OCC_ABNORMAL_MS {
                    // 判冻（§8.1）：pending>0 持续 ≥5s 且无净下降
                    if try_unfreeze(handle, deadline)? {
                        self.unfroze = true;
                        self.since = None; // 解冻成功：重开窗口继续写
                    } else {
                        return Err(
                            "目标终端输入冻结（选择模式），自动解冻失败——请点一下该终端窗口后重试"
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// 采样一次占用并推进判冻窗口（块写入后 / 122·Ok(0) 重试睡眠后的独立查询点；
    /// 纯查询不睡眠，不改变既有块间节奏）。
    fn sample(&mut self, handle: HANDLE, deadline: Instant) -> Result<(), String> {
        let pending = query_pending(handle)?;
        self.observe(pending, handle, deadline)
    }
}

/// 冻结自愈（Task 4，§8.1「背压与异常」行；M6R F1 实证：宿主选择模式冻结 =
/// 写入成功 TUI 读不到，排队回车丢提交语义）：向**目标控制台宿主窗口**
/// PostMessage WM_KEYDOWN/WM_KEYUP VK_ESCAPE 令宿主退出选择模式、输入缓冲恢复
/// 消费，再轮询占用确认。
///
/// 锁纪律：调用点在 [`CONSOLE_OP`] 锁内 + [`AttachGuard`] 作用域内（附加态保证
/// `GetConsoleWindow` 返回目标控制台宿主窗）；本函数**不得再取锁**（std Mutex
/// 不可重入，嵌套即死锁）。等待上限 2s，实际取 `min(2s, deadline 剩余)`——
/// deadline 耗尽由调用方既有循环顶部超时检查报错，本函数不另行超时报错。
///
/// 返回 `Ok(true)` = 已解冻（占用出现净下降或归零）；`Ok(false)` = 等待超时
/// 无下降（仍冻结，PostMessage 失败亦归此态）；`Err` 仅保留占用查询失败
/// （控制台已失效）路径。WT 宿主下冻结恢复未验证（§8.3 实机复验项）。
fn try_unfreeze(handle: HANDLE, deadline: Instant) -> Result<bool, String> {
    // SAFETY: FFI 调用；须处于目标控制台附加态（调用点由 AttachGuard 保证）
    let hwnd = unsafe { GetConsoleWindow() };
    if hwnd.0 == 0 {
        log::warn!("try_unfreeze：GetConsoleWindow 返回无效窗口（附加态缺失？）");
        return Ok(false);
    }
    // 发帖时占用基线（净下降判定起点）
    let baseline = query_pending(handle)?;
    // SAFETY: FFI 调用；hwnd 为刚取回的目标宿主窗，lparam 为本地常量（标准键消息布局）
    unsafe {
        if let Err(e) = PostMessageW(
            hwnd,
            WM_KEYDOWN,
            WPARAM(VK_ESCAPE.0 as usize),
            LPARAM(ESC_LPARAM_DOWN),
        ) {
            log::warn!(
                "try_unfreeze：PostMessage WM_KEYDOWN 失败（0x{:08X}）",
                e.code().0 as u32
            );
            return Ok(false);
        }
        if let Err(e) = PostMessageW(
            hwnd,
            WM_KEYUP,
            WPARAM(VK_ESCAPE.0 as usize),
            LPARAM(ESC_LPARAM_UP),
        ) {
            log::warn!(
                "try_unfreeze：PostMessage WM_KEYUP 失败（0x{:08X}）",
                e.code().0 as u32
            );
            return Ok(false);
        }
    }
    // 等 drain ≤2s：出现净下降（< 发帖时 baseline）或归零 → 解冻成功
    let wait_end = (Instant::now() + Duration::from_millis(UNFREEZE_WAIT_MS)).min(deadline);
    loop {
        let pending = query_pending(handle)?;
        if pending == 0 || pending < baseline {
            return Ok(true);
        }
        if Instant::now() >= wait_end {
            return Ok(false);
        }
        sleep(DRAIN_POLL_GAP);
    }
}

/// 背压排空（§8.1）：轮询目标输入缓冲占用直至 ≤ `families::DRAIN_TO`(40) 再继续；
/// 15ms 步距（[`DRAIN_POLL_GAP`]），同样受 `deadline` 约束（P2-1 真总预算）。
/// Task 4：每次轮询顺带喂 [`OccWatch`] 判冻监控（复用同一次占用查询，无额外
/// 系统调用；判冻触发才可能拉长等待，属自愈语义而非节奏改变）。
fn drain_to(handle: HANDLE, deadline: Instant, watch: &mut OccWatch) -> Result<(), String> {
    loop {
        let pending = query_pending(handle)?;
        watch.observe(pending, handle, deadline)?;
        if pending <= families::DRAIN_TO {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("注入超时（总预算耗尽），请重试".into());
        }
        sleep(DRAIN_POLL_GAP);
    }
}

/// 分片写入主循环（M9R §8.1 配方，取代旧 M6 消费墙保守实现）：
/// - 每块 `families::CHUNK_CHARS * 2` 事件（80 字符 = 160 事件单次写入单元）；
/// - **每轮循环顶部查 deadline**（P2-1 真总预算：写入/回车/122 重试/背压轮询共享）
///   耗尽 → `Err("注入超时（总预算耗尽），请重试")`；
/// - M6R 纪律：实写 < 分片长 → **从偏移续写**（续写不睡眠）；`Ok(0)` 不应发生
///   （防御），按 122 同款可重试处理；
/// - Err(122) → 100ms 重试同块（受 deadline）；其他错误 → 带错误码中文错误；
/// - 非背压：块间 `families::CHUNK_GAP_MS`(50ms)；背压：每块后 [`drain_to`]；
/// - Task 4 占用监控：每块写入后 / 背压轮询处（[`drain_to`] 内复用查询）/
///   122·Ok(0) 重试睡眠后采样输入缓冲占用（[`OccWatch`]）——判冻触发 →
///   [`try_unfreeze`] 自愈（成功置 [`PacedOutcome::unfroze`] 继续写；失败报中文
///   冻结错误）。采样为纯查询不睡眠，不改变既有块间节奏。
///
/// 返回 [`PacedOutcome`]（实写事件总数 + 是否解冻过）。
fn paced_write(
    handle: HANDLE,
    records: &[KeyRecordSpec],
    backpressure: bool,
    deadline: Instant,
) -> Result<PacedOutcome, String> {
    let chunk_events = families::CHUNK_CHARS * 2; // 80 字符 = 160 事件（§8.1）
    let mut sent = 0usize; // 已实写事件数（部分写续写的游标）
    let mut watch = OccWatch::new();
    while sent < records.len() {
        // P2-1：真总预算检查（写入/重试/轮询全走这里，一处收口）
        if Instant::now() >= deadline {
            return Err("注入超时（总预算耗尽），请重试".into());
        }
        let end = (sent + chunk_events).min(records.len());
        let chunk: Vec<FlatKeyRecord> =
            records[sent..end].iter().map(FlatKeyRecord::from).collect();
        match unsafe { write_chunk(handle, &chunk) } {
            Ok(written) if written as usize >= chunk.len() => {
                sent = end;
                if backpressure {
                    // 背压：每块后排空目标输入缓冲至阈值再继续（含末块，§8.1）——
                    // 轮询处同步喂判冻监控（复用查询）
                    drain_to(handle, deadline, &mut watch)?;
                } else {
                    if sent < records.len() {
                        // 非背压：块间短睡眠，给目标 TUI 留输入缓冲消费窗口
                        sleep(Duration::from_millis(families::CHUNK_GAP_MS));
                    }
                    // 每块写入后采样判冻（Task 4，§8.1；含末块，纯查询不睡眠）
                    watch.sample(handle, deadline)?;
                }
            }
            Ok(written) => {
                if written == 0 {
                    // Ok(0) 不应发生（防御）：按 122 同款 100ms 重试同块，睡后采样
                    sleep(RETRY_GAP);
                    watch.sample(handle, deadline)?;
                } else {
                    // M6R 纪律：实写 < 分片长，从偏移续写（续写不睡眠）
                    sent += written as usize;
                }
            }
            Err(e) if is_insufficient_buffer(&e) => {
                // 122 现语义（M6R 推翻消费墙）：目标读武装期缓冲暂满，可重试——
                // 100ms 后重试同块（§8.1）；deadline 由循环顶部统一把守；
                // 睡后采样判冻（Task 4：重试等待期同步监控消费进展）
                sleep(RETRY_GAP);
                watch.sample(handle, deadline)?;
            }
            Err(e) => {
                return Err(format!(
                    "WriteConsoleInput 失败（0x{:08X}）",
                    e.code().0 as u32
                ));
            }
        }
    }
    Ok(PacedOutcome {
        written: sent,
        unfroze: watch.unfroze,
    })
}

/// 对 pid 完整注入一条写入闭包（M9R 重写，取代旧 records 直传版）。
/// **锁纪律：调用方须已持 [`CONSOLE_OP`] 锁**（本函数为无锁内部版，doc 即契约：
/// 公共入口各取一次锁、全程单临界区；本函数与 [`resolve_target`] 都不再取锁，
/// 杜绝 std Mutex 不可重入导致的嵌套死锁）。
/// 流程：resolve_target（锁内探测附加+祖先回退，每步失败即 FreeConsole 复位）
/// → attach(target) → [`AttachGuard`] → open_conin → 写入闭包 → CloseHandle
/// → guard Drop 复位。闭包在 guard 作用域内执行、不早退出；任何错误/panic
/// 路径由 guard Drop 兜底复位（P1-1，CTRL_CLOSE_EVENT 防连带终止）。
/// 泛型 `T`（Task 4）：闭包可携带结果出临界区（text 路径传出 unfroze 旗标）。
fn inject_via<T>(pid: u32, write: impl FnOnce(HANDLE) -> Result<T, String>) -> Result<T, String> {
    let target = resolve_target(pid)?;
    attach(target).map_err(|code| format!("AttachConsole(pid={target}) 失败（0x{code:08X}）"))?;
    let _guard = AttachGuard;
    // SAFETY: FFI 调用；句柄生命周期收敛于本函数（下方无条件 CloseHandle）
    let handle = unsafe { open_conin() }?;
    let result = write(handle);
    // SAFETY: FFI 调用；handle 为本函数刚打开的内核句柄
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
    // _guard 在此 Drop → FreeConsole 复位（无论成败；幂等，M6 探测实证）
}

/// 文本注入（「打字 + 回车」铁则，M9R spec 感知版）：正文按 [`text_records`]
/// 分流构造（ASCII 走 VK 形态、非 ASCII 走 vk=0 字符流）+ 尾部 VK 形态回车
/// 事件对（固定 [`families::SUBMIT_DELAY_MS`] 后单批提交）。自适应节流与真总
/// 预算由族规格驱动（§8.1 / P2-1）。
/// `text` 已是 compose 后单行（含字面 \n 两字符场景也按字符事件直发）。
pub(crate) fn inject_text_spec(
    pid: u32,
    text: &str,
    spec: &FamilySpec,
) -> Result<InjectStats, String> {
    // P1-2：进程级串行（附加态全局唯一）；毒锁就地恢复（前次 panic 不放大为死锁）
    let _lock = CONSOLE_OP.lock().unwrap_or_else(|e| e.into_inner());
    // 域内路径：锁内 resolve_target（探测附加+祖先回退全在锁内，杜绝并发附加竞态）
    let chars = text.chars().count();
    let bp = families::use_backpressure(spec, chars);
    // P2-1：真总预算（背压按斜率放宽）；取锁后起算（预算度量注入而非等锁）
    let deadline = Instant::now() + Duration::from_millis(families::inject_budget_ms(spec, chars));
    let layout = WinKeyLayout;
    let body = text_records(text, &layout);
    let enter = enter_records(&layout);
    // unfroze 透传（Task 4）：正文/回车任一块写入途中解冻成功即置位。
    // 「解冻成功补发回车」的等价实现（§8.1 定案的补发针对的是**冻结吞掉的提交
    // 回车**）：正文块写入途中解冻成功 → 既有流程的回车块照常发送即达成补发
    // 语义；回车块写入前/写入中触发解冻成功 → 同理照常发送。故**不新增独立
    // 补发逻辑**（避免双回车）；回车块因解冻失败而 Err → 既有半成功文案兜底。
    let unfroze = inject_via(pid, move |handle| {
        let body_out = paced_write(handle, &body, bp, deadline)?;
        // 提交回车：正文写完固定延迟后单批发（回车不进 chunk 计划，§8.1）
        sleep(Duration::from_millis(families::SUBMIT_DELAY_MS));
        let enter_out = paced_write(handle, &enter, false, deadline).map_err(|e| {
            // 半成功防重（对齐 macOS tmux「文本已入 pane」先例）：正文已进输入行、
            // 回车未提交，盲目重试会重复正文——追加人工确认提示
            format!("{e}；正文可能已写入输入行，重试将重复——建议先人工确认终端")
        })?;
        Ok(body_out.unfroze || enter_out.unfroze)
    })?;
    Ok(InjectStats {
        written: chars, // Ok = 全量送达；written 记正文字符数（不含提交回车）
        backpressure: bp,
        unfroze,
    })
}

/// A 族方向键 → VT 序列映射表（本地常量，M6R §8.1 定案：ESC [ + A/B/C/D 字母流，
/// vk=0 字符形态整条单批原子写）。
/// 探针派生脆弱常量——版本复验清单见 `super::families`（宪法横切 6 集中落点），
/// 勿就地改值。
const ARROW_VT_SEQS: [(&str, &str); 4] = [
    ("up", "\x1b[A"),
    ("down", "\x1b[B"),
    ("right", "\x1b[C"),
    ("left", "\x1b[D"),
];

/// 键名 → 键事件序列（域判定纯构造，P2-2）：控制键/单字符字母数字走
/// [`control_records`]；方向键按族分支——A 族 [`vt_seq_records`]（VT 字符流）/
/// B 族 [`vk_arrow_records`]（VK+scan）。域外 → `None`。
fn key_records_for(key: &str, spec: &FamilySpec) -> Option<Vec<KeyRecordSpec>> {
    if let Some(records) = control_records(key, &WinKeyLayout) {
        return Some(records);
    }
    let vt = ARROW_VT_SEQS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, seq)| *seq);
    match (vt, spec.family) {
        (Some(seq), TuiFamily::RawVt) => Some(vt_seq_records(seq)),
        (Some(_), TuiFamily::Crossterm) => vk_arrow_records(key, &WinKeyLayout),
        (None, _) => None, // 域外
    }
}

/// 单键注入（M9R spec 感知版）：键域内（enter/esc/tab/单字符字母数字/方向键）
/// → 按族形态单批写入；域外 → **快速失败报错，不回退文本+回车**（P2-2 闭环）。
/// 域校验在取锁/附加之前（假 pid 也不触发任何控制台附加）。
pub(crate) fn inject_key_spec(pid: u32, key: &str, spec: &FamilySpec) -> Result<(), String> {
    // P2-2：域校验先行——必须在取锁/附加之前快速失败（不触任何控制台 API）
    let Some(records) = key_records_for(key, spec) else {
        return Err(format!(
            "不支持的按键：{key}（域：enter/esc/tab/单字符字母数字/方向键）"
        ));
    };
    let _lock = CONSOLE_OP.lock().unwrap_or_else(|e| e.into_inner());
    // 键事件 ≤ 8 条（方向键 VT 序列），无需放宽——固定基础预算（§8.1）
    let deadline = Instant::now() + Duration::from_millis(families::BASE_BUDGET_MS);
    inject_via(pid, move |handle| {
        paced_write(handle, &records, false, deadline)?;
        Ok(())
    })
}

/// 输入行尾部屏读（Task 4，M6R/§8.1：注入后读屏可判定「文本滞留输入行」）。
/// 契约签名 `read_input_tail(pid, n)`：CONOUT$ + `ReadConsoleOutputCharacterW`
/// 从光标处**同行向前**读 `min(n, cursor_x+1)` 个 UTF-16 unit，
/// `String::from_utf16_lossy` 返回。**不 trim**——截尾/匹配语义归调用方
/// （直发滞留回查与 Task 5 插队草稿确认用）。
///
/// 锁纪律：全程与注入共用 [`CONSOLE_OP`] 单临界区（毒锁恢复同既有）——
/// 锁 → [`resolve_target`]（无锁内部版，调用方持锁）→ attach → [`AttachGuard`]
/// → CONOUT$ 读 → CloseHandle → guard Drop 复位 → 解锁。
#[allow(dead_code)] // Task 5 确认层消费；本任务先落契约签名（实机探测见 #[ignore] 测试）
pub(crate) fn read_input_tail(pid: u32, n: usize) -> Result<String, String> {
    let _lock = CONSOLE_OP.lock().unwrap_or_else(|e| e.into_inner());
    let target = resolve_target(pid)?;
    attach(target).map_err(|code| format!("AttachConsole(pid={target}) 失败（0x{code:08X}）"))?;
    let _guard = AttachGuard;
    // SAFETY: FFI 调用；句柄生命周期收敛于本函数（下方无条件 CloseHandle）
    let handle = unsafe { open_conout() }?;
    let result = read_tail_chars(handle, n);
    // SAFETY: FFI 调用；handle 为本函数刚打开的内核句柄
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
    // _guard 在此 Drop → FreeConsole 复位；_lock 在此释放
}

/// 屏读实现体（[`read_input_tail`] 已开 CONOUT$ 句柄）：`GetConsoleScreenBufferInfo`
/// 取光标位置 → 同行向前读 `min(n, cursor_x+1)` 个 unit。
#[allow(dead_code)] // 同 read_input_tail（Task 5 消费）
fn read_tail_chars(handle: HANDLE, n: usize) -> Result<String, String> {
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: FFI 调用；info 为本函数栈上缓冲
    unsafe { GetConsoleScreenBufferInfo(handle, &mut info) }.map_err(|e| {
        format!(
            "GetConsoleScreenBufferInfo 失败（0x{:08X}）",
            e.code().0 as u32
        )
    })?;
    let cursor = info.dwCursorPosition;
    // 同行向前：起点 X = cursor.X - (want-1)，want 封顶 cursor.X+1（到行首）；
    // 再封顶 i16::MAX 防御（cursor.X 理论上界，缓冲宽度实际远小于此）
    let want = n.min(cursor.X.max(0) as usize + 1).min(i16::MAX as usize);
    let mut buf = vec![0u16; want];
    let mut read = 0u32;
    let start = COORD {
        X: cursor.X - (want as i16 - 1),
        Y: cursor.Y,
    };
    // SAFETY: FFI 调用；buf 长度即读取上限（windows-rs 绑定以切片长度为 nLength）
    unsafe { ReadConsoleOutputCharacterW(handle, &mut buf, start, &mut read) }.map_err(|e| {
        format!(
            "ReadConsoleOutputCharacterW 失败（0x{:08X}）",
            e.code().0 as u32
        )
    })?;
    buf.truncate(read as usize);
    Ok(String::from_utf16_lossy(&buf)) // 不 trim（契约：截尾/匹配语义归调用方）
}

/// 无族回退规格（本地常量，对齐计划「无族按快消费者默认」口径）：A 族 RawVt +
/// 非慢消费者 + 5s 确认超时。仅供旧薄壳 [`locate_and_inject`] /
/// [`locate_and_send_key`]（trait 默认路径与既有调用）使用；正规路径一律经
/// queue 驱动把族规格送达 `locate_and_*_spec`。
const DEFAULT_SPEC: FamilySpec = FamilySpec {
    family: TuiFamily::RawVt,
    verified_with: "default-fast",
    slow_consumer: false,
    confirm_timeout_ms: 5_000,
};

/// 旧薄壳（保留供 trait 默认路径与既有调用编译）：无族回退 [`DEFAULT_SPEC`]
/// + 委托 [`inject_text_spec`]。
pub(crate) fn locate_and_inject(pid: u32, text: &str) -> Result<(), String> {
    inject_text_spec(pid, text, &DEFAULT_SPEC).map(|_| ())
}

/// 旧薄壳（单键）：无族回退 [`DEFAULT_SPEC`] + 委托 [`inject_key_spec`]。
pub(crate) fn locate_and_send_key(pid: u32, key: &str) -> Result<(), String> {
    inject_key_spec(pid, key, &DEFAULT_SPEC)
}

/// PID 策略（M6 裁定）：先试 pid 本体（会话 CLI 原生进程 claude.exe/codex.exe/kimi.exe）；
/// `AttachConsole` 失败 → 祖先链（近→远）第一个附加成功者；全部失败 → 中文错误。
/// M9R 锁纪律：**无锁内部版**——假定调用方已持 [`CONSOLE_OP`] 锁（公共入口
/// 各取一次锁、全程单临界区；resolve 与注入不得各自加锁嵌套）。探测性附加/
/// 祖先回退每一步失败都 FreeConsole 复位，任何路径不滞留附加态。
fn resolve_target(pid: u32) -> Result<u32, String> {
    let mut tried: Vec<String> = Vec::new();
    // 探测性附加：命中即复位（inject_via 会重新附加）——任何路径都不滞留附加态
    if let Err(code) = attach(pid) {
        tried.push(format!("pid={pid} 0x{code:08X}"));
    } else {
        let _ = unsafe { FreeConsole() };
        return Ok(pid);
    }
    let system = sysinfo::System::new_all();
    for ancestor in crate::window::win32::collect_ancestor_pids(&system, pid) {
        if ancestor == pid {
            continue; // 本体已试过
        }
        if let Err(code) = attach(ancestor) {
            tried.push(format!("ancestor={ancestor} 0x{code:08X}"));
        } else {
            let _ = unsafe { FreeConsole() };
            return Ok(ancestor);
        }
    }
    Err(format!(
        "附加目标控制台失败（pid={pid}，尝试：{}）",
        tried.join("；")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P2-2 键域校验单测：域外键报错（不回退文本+回车），且**域校验必须在
    /// attach 之前快速失败**——假 pid 424242 不触发任何控制台附加（无真控制台
    /// 也可稳定跑，常规 cargo test 门禁内执行）。
    #[test]
    fn unknown_key_rejected() {
        let err =
            inject_key_spec(424242, "bad!", &families::family_for("codex").unwrap()).unwrap_err();
        assert!(err.contains("不支持的按键"));
    }

    /// 族分派单测（Minor 4）：同一方向键名按族规格分流——A 族（claude，RawVt）
    /// 走 VT 字符流（vk=0 整条单批原子写），B 族（codex，Crossterm）走 VK+scan
    /// 键形态（VK_UP=0x26）。
    #[test]
    fn key_records_for_family_dispatch() {
        let claude = families::family_for("claude").unwrap();
        let up_vt = key_records_for("up", &claude).unwrap();
        assert!(up_vt.iter().all(|k| k.vk == 0)); // VT 字符流形态
        let codex = families::family_for("codex").unwrap();
        let up_vk = key_records_for("up", &codex).unwrap();
        assert_eq!(up_vk[0].vk, 0x26); // VK_UP 键形态
    }

    /// WinKeyLayout 真 FFI 契约单测（无需目标控制台，常规 cargo test 可跑）：
    /// 不硬编码键位（规避键盘布局差异误报）——vk_of('a') 与 `VkKeyScanW('a')`
    /// 低字节直接对拍；另钉三条铁律：'\r' → VK_RETURN 契约、非 ASCII → 0
    /// 字符流口径、扫描码派生非 0。
    #[test]
    fn win_key_layout_ffi_contract() {
        let layout = WinKeyLayout;
        // 字符键：vk_of 与 VkKeyScanW 低字节对拍（非硬编码 0x41，布局无关）
        let vk_scan_a = unsafe { VkKeyScanW('a' as u16) };
        assert_ne!(vk_scan_a, -1); // 'a' 在真实键盘布局必可键入
        assert_eq!(layout.vk_of('a'), (vk_scan_a & 0xFF) as u16);
        // KeyLayout trait 契约：'\r' 必返 VK_RETURN(0x0D)
        assert_eq!(layout.vk_of('\r'), 0x0D);
        // 非 ASCII 一律 0（纯字符流口径，与 ConIn.ps1 一致）
        assert_eq!(layout.vk_of('我'), 0);
        // 扫描码派生：VK_RETURN 对应扫描码非 0
        assert!(layout.scan_of(0x0D) > 0);
    }

    /// Task 12 真 FFI 一跳集成测试（#[ignore]：需要弹真实 conhost 窗口，不在常规门禁跑）。
    /// 运行：`cargo test --lib inject::windows_console -- --ignored --nocapture`
    ///
    /// 拓扑口径（M6 实证 + 本机对照实验 2026-09-18）：测试进程直生 cmd（CREATE_NEW_CONSOLE）
    /// 的 AttachConsole 返回 0x80070005，而 conhost.exe cmd /k（PowerShell Start-Process
    /// 起手）拓扑下 attach cmd.exe 稳定成功——故本测试 shell 出 PowerShell 建立已证拓扑。
    /// 地面真值：向该 cmd 注入 `echo FFI-HOP-OK > hop.txt`，文件落盘 = 送达。
    #[test]
    #[ignore = "需要真实 conhost 窗口与外部进程，实机验证时以 --ignored 运行"]
    fn ffi_hop_injects_into_fresh_console_cmd() {
        use super::locate_and_inject;
        use std::process::Command;

        let work = std::env::temp_dir().join(format!("mam-ffi-hop-{}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let hop = work.join("hop.txt");
        let _ = std::fs::remove_file(&hop);

        // 已证拓扑：conhost.exe cmd /k（挂在 work 目录）
        let script = format!(
            "Start-Process conhost.exe -ArgumentList 'cmd.exe','/k' -WorkingDirectory '{}'",
            work.display()
        );
        let st = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status()
            .expect("启动 powershell 失败");
        assert!(st.success(), "Start-Process 失败");
        std::thread::sleep(std::time::Duration::from_millis(3000));

        // 目标 pid：最新起的裸 cmd.exe（CommandLine 恰为 "cmd.exe"，排除其它业务进程）。
        // 查询落盘为 .ps1 执行——规避 Rust→PowerShell 的多层引号转义
        let finder = work.parent().unwrap().join("mam-ffi-hop-find.ps1");
        std::fs::write(
            &finder,
            // CommandLine 实为 "cmd.exe /k"（本测试独有形态，Exclude 其它业务进程）
            "(Get-CimInstance Win32_Process -Filter \"Name='cmd.exe'\" | Where-Object { $_.CommandLine -match 'cmd\\.exe /k$' } | Sort-Object CreationDate -Descending | Select-Object -First 1).ProcessId",
        )
        .unwrap();
        let find_pid = Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &finder.display().to_string(),
            ])
            .output()
            .expect("枚举 cmd pid 失败");
        let out = String::from_utf8_lossy(&find_pid.stdout).trim().to_string();
        let _ = std::fs::remove_file(&finder);
        println!("ffi-hop: target cmd pid={out}");
        let pid: u32 = out.parse().expect("cmd pid 解析失败");

        // 等 cmd 提示符就绪（读武装）
        std::thread::sleep(std::time::Duration::from_millis(1500));

        // 真 FFI 一跳：打字 + 回车（echo 重定向落盘）
        let result = locate_and_inject(pid, "echo FFI-HOP-OK > hop.txt");
        println!("ffi-hop: inject result={result:?}");

        // 等 cmd 执行落盘
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let content = std::fs::read_to_string(&hop).ok();
        println!("ffi-hop: hop.txt={content:?}");

        // 收尾：关掉探测窗口（按 pid 杀 cmd 与其 conhost 宿主）
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();

        assert!(result.is_ok(), "注入应成功：{result:?}");
        assert!(
            content.map(|c| c.contains("FFI-HOP-OK")).unwrap_or(false),
            "hop.txt 应含 FFI-HOP-OK（地面真值）"
        );
    }

    // ===== Task 4 实机探测（占用监控 / 冻结自愈 / 屏读层）=====
    // 两条测试均涉及真实控制台子进程（弹真实 conhost 窗口），按批次全局约束
    // 一律 #[ignore]——常规门禁只编译；实机真跑归 Task 12。测试只碰 temp 目录
    // 探测会话，结束 taskkill 清场，零接触真实 ~/.mam。

    /// 探测会话起手（复用 [`ffi_hop_injects_into_fresh_console_cmd`] 已证拓扑与
    /// pid 查找法，temp 目录）：PowerShell Start-Process conhost.exe cmd /k →
    /// 查找最新起的裸 cmd.exe（CommandLine 恰为 "cmd.exe /k"，本仓探测独有
    /// 形态，Exclude 其它业务进程）。pid 查询落盘 .ps1 执行——规避
    /// Rust→PowerShell 的多层引号转义（ffi_hop 实证做法）。
    fn spawn_probe_cmd(work: &std::path::Path, tag: &str) -> u32 {
        use std::process::Command;
        let script = format!(
            "Start-Process conhost.exe -ArgumentList 'cmd.exe','/k' -WorkingDirectory '{}'",
            work.display()
        );
        let st = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status()
            .expect("启动 powershell 失败");
        assert!(st.success(), "Start-Process 失败");
        std::thread::sleep(Duration::from_millis(3000));

        let finder = work.parent().unwrap().join(format!("mam-{tag}-find.ps1"));
        std::fs::write(
            &finder,
            "(Get-CimInstance Win32_Process -Filter \"Name='cmd.exe'\" | Where-Object { $_.CommandLine -match 'cmd\\.exe /k$' } | Sort-Object CreationDate -Descending | Select-Object -First 1).ProcessId",
        )
        .unwrap();
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &finder.display().to_string(),
            ])
            .output()
            .expect("枚举 cmd pid 失败");
        let _ = std::fs::remove_file(&finder);
        let pid_text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        println!("{tag}: target cmd pid={pid_text}");
        pid_text.parse().expect("cmd pid 解析失败")
    }

    /// 清场：按 pid 连带杀 cmd 与其 conhost 宿主（ffi_hop 同款）。
    fn kill_probe(pid: u32) {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }

    /// Task 4 冻结自愈探测（#[ignore]：需要真实 conhost 子进程，实机真跑归
    /// Task 12）。运行：`cargo test --lib inject::windows_console -- --ignored --nocapture`
    ///
    /// 流程：conhost cmd /k 拓扑 → 内部通道快速连写大批事件制造占用（背压模式
    /// 块后排空，cmd 消费不及时即 pending>0）→ 调 [`try_unfreeze`] → 断言
    /// 「占用归零/有净下降 **或** 返回含中文冻结错误」二选一放宽——真冻结态依赖
    /// 宿主选择模式（写入成功 TUI 读不到），本探测无法确定性合成，真值验证归
    /// Task 12（§8.1 / §8.3）。
    #[test]
    #[ignore = "需要真实 conhost 窗口与外部进程，实机验证时以 --ignored 运行"]
    fn occ_stuck_triggers_unfreeze_probe() {
        let work = std::env::temp_dir().join(format!("mam-occ-probe-{}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let pid = spawn_probe_cmd(&work, "occ");
        // 等 cmd 提示符就绪（读武装）
        std::thread::sleep(Duration::from_millis(1500));

        // 单线程测试直调内部通道（无并发，锁纪律不适用）：
        // resolve → attach → guard → open_conin → 背压连写 4000 字符制造占用
        let target = resolve_target(pid).expect("resolve 目标失败");
        attach(target).expect("attach 目标失败");
        let guard = AttachGuard;
        // SAFETY: 测试 FFI；句柄在下方无条件 CloseHandle
        let handle = unsafe { open_conin() }.expect("打开 CONIN$ 失败");
        let text = "x".repeat(4000);
        let records = text_records(&text, &WinKeyLayout);
        let outcome = paced_write(
            handle,
            &records,
            true,
            Instant::now() + Duration::from_secs(120),
        );
        println!("occ-probe: paced_write={outcome:?}");
        // 放宽项 b) 之一：连写途中即报中文冻结错误（真冻结且自愈失败路径）
        let write_freeze_err = outcome
            .as_ref()
            .err()
            .map(|e| e.contains("冻结"))
            .unwrap_or(false);
        let before = query_pending(handle).unwrap_or(0);
        println!("occ-probe: pending before={before}");

        // 冻结自愈：PostMessage ESC → 等 drain ≤2s（探测 deadline 给足 10s）
        let unfroze = try_unfreeze(handle, Instant::now() + Duration::from_secs(10));
        println!("occ-probe: try_unfreeze={unfroze:?}");
        let after = query_pending(handle).unwrap_or(0);
        println!("occ-probe: pending after={after}");

        // SAFETY: FFI 收尾
        unsafe {
            let _ = CloseHandle(handle);
        }
        drop(guard); // FreeConsole 复位后再清场
        kill_probe(pid);

        // 二选一放宽断言：a) 占用归零/有净下降（自愈生效或消费者自然消化）；或
        // b) 返回含中文冻结语义错误（§8.3 WT 宿主未验证由文案覆盖）
        let drained = after == 0 || after < before;
        let unfreeze_chinese_err = unfroze
            .as_ref()
            .err()
            .map(|e| e.contains("冻结"))
            .unwrap_or(false);
        assert!(
            drained || write_freeze_err || unfreeze_chinese_err,
            "应「占用归零/净下降」或「返回含中文冻结错误」：before={before} after={after} \
             paced_write={outcome:?} try_unfreeze={unfroze:?}"
        );
    }

    /// Task 4 屏读层探测（#[ignore]：需要真实 conhost 子进程，实机真跑归
    /// Task 12）。同拓扑 → 不带回车直发文本（tests 模块可直接调私有
    /// [`paced_write`] / [`text_records`]：attach→guard→open_conin→paced_write
    /// 不发 enter）→ [`read_input_tail`]\(pid, 16) 断言返回串含 "tail123"
    /// （屏读滞留输入行 = Task 5 确认层的确切用例）→ 补发 enter 清行 →
    /// taskkill 清场。
    #[test]
    #[ignore = "需要真实 conhost 窗口与外部进程，实机验证时以 --ignored 运行"]
    fn read_input_tail_returns_recent_chars() {
        let work = std::env::temp_dir().join(format!("mam-tail-probe-{}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let pid = spawn_probe_cmd(&work, "tail");
        // 等 cmd 提示符就绪（读武装）
        std::thread::sleep(Duration::from_millis(1500));

        // 阶段①：直发文本，不发 enter（输入行滞留 = 屏读对象）
        {
            let target = resolve_target(pid).expect("resolve 目标失败");
            attach(target).expect("attach 目标失败");
            let guard = AttachGuard;
            // SAFETY: 测试 FFI；句柄在下方无条件 CloseHandle
            let handle = unsafe { open_conin() }.expect("打开 CONIN$ 失败");
            let records = text_records("echo tail123", &WinKeyLayout);
            let outcome = paced_write(
                handle,
                &records,
                false,
                Instant::now() + Duration::from_secs(10),
            )
            .expect("直发文本失败");
            assert_eq!(outcome.written, records.len(), "全文应一次写完");
            // SAFETY: FFI 收尾
            unsafe {
                let _ = CloseHandle(handle);
            }
            drop(guard); // FreeConsole 复位——read_input_tail 需全新附加（单临界区互斥）
        }

        // 命中回显渲染余量
        std::thread::sleep(Duration::from_millis(500));

        // 阶段②：屏读（契约签名：自取 CONSOLE_OP 锁、自附加、CONOUT$ 读尾部）
        let tail = read_input_tail(pid, 16).expect("read_input_tail 失败");
        println!("tail-probe: read_input_tail(16)={tail:?}");
        assert!(
            tail.contains("tail123"),
            "输入行尾部屏读应含 tail123：{tail:?}"
        );

        // 阶段③：补发 enter 清行（提交滞留行，防探测残留挂输入行）
        {
            let target = resolve_target(pid).expect("resolve 目标失败(清行)");
            attach(target).expect("attach 目标失败(清行)");
            let guard = AttachGuard;
            // SAFETY: 测试 FFI；句柄在下方无条件 CloseHandle
            let handle = unsafe { open_conin() }.expect("打开 CONIN$ 失败(清行)");
            let enter = enter_records(&WinKeyLayout);
            paced_write(
                handle,
                &enter,
                false,
                Instant::now() + Duration::from_secs(10),
            )
            .expect("补发 enter 失败");
            // SAFETY: FFI 收尾
            unsafe {
                let _ = CloseHandle(handle);
            }
            drop(guard);
        }

        kill_probe(pid);
    }
}
