#![cfg(windows)]
//! Windows ConPTY 注入通道（M9）：`AttachConsole` → 打开 `CONIN$` →
//! `WriteConsoleInput` 逐片写键事件。M6 探测两处实证修正（落地必须照做）：
//! 1. AttachConsole 后 `GetStdHandle` 返回旧控制台陈旧句柄（stdin 为管道时必败）
//!    ——必须以 `CONIN$` 显式打开；
//! 2. `INPUT_RECORD` 必须拍平为 blittable 布局（嵌套结构体在 FFI 封送报
//!    0x8007007A）——本文件自定 20 字节平铺结构指针直传，与 ConIn.ps1 的
//!    C# 平铺完全同构。
//!
//! 消费墙（M6 核心发现，引擎内建应对）：目标 TUI 读武装期首读预算 13~43 字符，
//! 超出 `WriteConsoleInput` 返回 ERROR_INSUFFICIENT_BUFFER(122)；退格事件永不被拦。
//! 引擎策略：小分片（每片 1 字符 = keydown+keyup 两事件）与片间短 sleep（15ms），
//! 失败重试（100ms 间隔，总预算 10 秒）；预算耗尽仍失败 → 中文错误。
//! 短消息（≤35 字符）在该策略下即时成功。

use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::core::Error as WinError;
use windows::core::HRESULT;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    AttachConsole, FreeConsole, WriteConsoleInputW, INPUT_RECORD, KEY_EVENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC};

use super::engine::{control_records, enter_records, text_records, KeyLayout, KeyRecordSpec};

/// 消费墙分片粒度：每片 1 字符 = keydown + keyup 两事件（M6 实证首读预算 13~43 字符）
const CHUNK_EVENTS: usize = 2;
/// 片间短 sleep：给目标 TUI 留出消费输入缓冲的窗口
const CHUNK_GAP: Duration = Duration::from_millis(15);
/// 122（消费墙）重试间隔
const RETRY_GAP: Duration = Duration::from_millis(100);
/// 注入总预算：耗尽即判「目标终端输入未就绪」
const INJECT_BUDGET: Duration = Duration::from_secs(10);

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

/// 单片写入：平铺结构切片按 `INPUT_RECORD` 同构布局指针直传（上方编译期断言锁定
/// 尺寸/对齐一致；windows-rs `WriteConsoleInputW` 内部对切片指针仅 transmute 直通，
/// 无嵌套封马 → 不踩 0x8007007A）。返回 windows Error 以便按错误码分流（122 = 消费墙）。
///
/// # SAFETY
/// FFI 调用：`handle` 须为有效 CONIN$ 句柄；切片生命周期覆盖调用全程。
unsafe fn write_chunk(handle: HANDLE, chunk: &[FlatKeyRecord]) -> Result<(), WinError> {
    let records = std::slice::from_raw_parts(chunk.as_ptr() as *const INPUT_RECORD, chunk.len());
    let mut written = 0u32;
    WriteConsoleInputW(handle, records, &mut written)
}

/// 消费墙判定：ERROR_INSUFFICIENT_BUFFER(122)（HRESULT 形态 0x8007007A）
fn is_insufficient_buffer(e: &WinError) -> bool {
    e.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0)
}

/// 分片写入主循环：每片 1 字符两事件；片间 15ms；Err(122) 100ms 重试整片；
/// 总预算 10s，耗尽 → 中文错误（M6 消费墙语义，可重试）。
fn write_all(handle: HANDLE, records: &[KeyRecordSpec]) -> Result<(), String> {
    let deadline = Instant::now() + INJECT_BUDGET;
    let mut sent = 0usize;
    while sent < records.len() {
        let end = (sent + CHUNK_EVENTS).min(records.len());
        let chunk: Vec<FlatKeyRecord> =
            records[sent..end].iter().map(FlatKeyRecord::from).collect();
        match unsafe { write_chunk(handle, &chunk) } {
            Ok(()) => {
                sent = end;
                if sent < records.len() {
                    sleep(CHUNK_GAP);
                }
            }
            Err(e) if is_insufficient_buffer(&e) => {
                // 消费墙：目标 TUI 读预算已满，等它消费后重试同一片
                if Instant::now() >= deadline {
                    return Err("目标终端输入未就绪（M6 消费墙），请稍后重试".into());
                }
                sleep(RETRY_GAP);
            }
            Err(e) => {
                return Err(format!(
                    "WriteConsoleInput 失败（0x{:08X}）",
                    e.code().0 as u32
                ));
            }
        }
    }
    Ok(())
}

/// 对已定位 pid 完整注入：attach → `CONIN$` → 分片写入 → `CloseHandle` →
/// `FreeConsole` 复位（收尾，无论成败都执行）。
///
/// FreeConsole 复位不可省（计划明文）：注入后滞留附加列表的进程，在宿主终端窗口
/// 关闭时会收到 CTRL_CLOSE_EVENT——本应用不装 SetConsoleCtrlHandler，默认行为是
/// **被系统终止**（用户关一个收过消息的终端 = MAM 整个应用跟着退出）。
fn inject_via(pid: u32, records: &[KeyRecordSpec]) -> Result<(), String> {
    attach(pid).map_err(|code| format!("AttachConsole(pid={pid}) 失败（0x{code:08X}）"))?;
    // SAFETY: FFI 调用；句柄生命周期收敛于本函数（下方无条件 CloseHandle）
    let handle = unsafe { open_conin() }?;
    let result = write_all(handle, records);
    // SAFETY: FFI 调用；handle 为本函数刚打开的内核句柄
    unsafe {
        let _ = CloseHandle(handle);
        // 复位附加态（幂等，M6 探测实证）：见函数 doc 的 CTRL_CLOSE_EVENT 风险
        let _ = FreeConsole();
    }
    result
}

/// 文本注入（「打字 + 回车」铁则）：正文按 [`text_records`] 分流构造
/// （ASCII 走 VK 形态、非 ASCII 走 vk=0 字符流）+ 尾部 VK 形态回车事件对。
/// `text` 已是 compose 后单行（含字面 \n 两字符场景也按字符事件直发）。
pub(crate) fn inject_text(target_pid: u32, text: &str) -> Result<(), String> {
    let mut records = text_records(text, &WinKeyLayout);
    // 尾部回车事件对：VK 形态（M9R 修正，取代旧 \n 特判 vk=0 形态——B 族丢
    // vk=0 控制字符）；与 macOS 通道「文本 + Enter 两连发」语义对齐
    records.extend(enter_records(&WinKeyLayout));
    inject_via(target_pid, &records)
}

/// 单键注入：键域内（enter/esc/tab/单字符字母数字）→ VK+scan 键事件对；
/// 域外（多字符/非 ASCII）→ 保持旧回退行为走文本通道（域外直接报错归 Task 3）。
pub(crate) fn inject_key(target_pid: u32, key: &str) -> Result<(), String> {
    match control_records(key, &WinKeyLayout) {
        Some(records) => inject_via(target_pid, &records),
        None => inject_text(target_pid, key),
    }
}

/// PID 策略（M6 裁定）：先试 pid 本体（会话 CLI 原生进程 claude.exe/codex.exe/kimi.exe）；
/// `AttachConsole` 失败 → 祖先链（近→远）第一个附加成功者；全部失败 → 中文错误。
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

/// 生产入口：定位可附加控制台并注入文本（打字 + 回车）。
pub(crate) fn locate_and_inject(pid: u32, text: &str) -> Result<(), String> {
    let target = resolve_target(pid)?;
    inject_text(target, text)
}

/// 生产入口：定位可附加控制台并发送单键。
pub(crate) fn locate_and_send_key(pid: u32, key: &str) -> Result<(), String> {
    let target = resolve_target(pid)?;
    inject_key(target, key)
}

#[cfg(test)]
mod tests {
    /// Task 15 真 FFI 一跳集成测试（#[ignore]：需要弹真实 conhost 窗口，不在常规门禁跑）。
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
}
