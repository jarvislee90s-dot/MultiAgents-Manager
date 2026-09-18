// 电源保活（M4 T3，宪法 D13）：远程开启期间阻止系统空闲睡眠与磁盘休眠（屏幕可熄）。
// macOS = caffeinate -ims 子进程（随 release kill）；Windows = 常驻线程
// SetThreadExecutionState(ES_CONTINUOUS|ES_SYSTEM_REQUIRED) + 磁盘休眠代设 0/还原
// （PowerReadACValueIndex/PowerWriteACValueIndex，代设前持久化原值——崩溃后下次启动还原）。
// 可测核 PowerCore 注入式状态机（同 PairingClock 先例）；OS 调用集中在 RealOps。
//
// 测试红线：单测只驱动 FakeOps（零网络、零真实 ~/.mam、绝不真起 caffeinate）；
// should_acquire 是纯函数。生产 RealOps 只在 start_server / 退出钩子路径触达。

use once_cell::sync::Lazy;

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

/// 状态机核（可测）：acquire 幂等 + 磁盘代设/还原 + 原值持久化。
/// P4 裁决（控制器 2026-09-16）：acquire **已持锁返回 true（幂等成功）**——
/// 「不重复 start」语义由 held 门锁定（重复 acquire 不再 exec_start），
/// 测试断言 started == ["start", "stop"] 即变异锚点
pub struct PowerCore<O: PowerOps> {
    ops: O,
    held: bool,
    saved_disk: Option<u32>,
}

impl<O: PowerOps> PowerCore<O> {
    pub fn new(ops: O) -> Self {
        Self {
            ops,
            held: false,
            saved_disk: None,
        }
    }

    /// 获取电源锁（幂等）：exec_start + 磁盘代设 0（代设前持久化原值）。
    /// 恒返回 true（未持锁则持锁，已持锁幂等成功——P4 裁决）
    pub fn acquire(&mut self) -> bool {
        if self.held {
            return true; // 幂等：不重复 start（P4 裁决）
        }
        self.ops.exec_start();
        if let Some(cur) = self.ops.disk_read() {
            if cur != 0 {
                // 已是「永不」则不动（还原也跳过）
                self.saved_disk = Some(cur);
                self.ops.persist_saved(Some(cur));
                self.ops.disk_set(Some(0));
            }
        }
        self.held = true;
        true
    }

    /// 释放电源锁（幂等）：exec_stop + 磁盘还原原值 + 清持久化键
    pub fn release(&mut self) {
        if !self.held {
            return;
        }
        self.ops.exec_stop();
        if let Some(v) = self.saved_disk.take() {
            self.ops.disk_set(Some(v));
            self.ops.persist_saved(None);
        }
        self.held = false;
    }

    /// 归还底层 ops（测试断言用）
    pub fn into_ops(self) -> O {
        self.ops
    }
}

// ============================================================
// 生产 RealOps（平台门控）与全局句柄
// ============================================================

struct RealOps;

#[cfg(target_os = "macos")]
impl PowerOps for RealOps {
    fn exec_start(&mut self) {
        real_start_mac();
    }
    fn exec_stop(&mut self) {
        real_stop_mac();
    }
    // macOS 磁盘休眠由 caffeinate -m 覆盖，无需代设
    fn disk_read(&mut self) -> Option<u32> {
        None
    }
    fn disk_set(&mut self, _: Option<u32>) {}
    fn persist_saved(&mut self, _: Option<u32>) {}
}

// Windows：SetThreadExecutionState 常驻线程 + 磁盘休眠代设（原值 KV 持久化）
#[cfg(windows)]
impl PowerOps for RealOps {
    fn exec_start(&mut self) {
        win::exec_start();
    }
    fn exec_stop(&mut self) {
        win::exec_stop();
    }
    fn disk_read(&mut self) -> Option<u32> {
        win::disk_read()
    }
    fn disk_set(&mut self, v: Option<u32>) {
        // None 恒不出现（PowerCore 还原恒带原值），防御性忽略
        if let Some(v) = v {
            win::disk_set(v);
        }
    }
    fn persist_saved(&mut self, v: Option<u32>) {
        // Some = 代设前原值（崩溃后 restore_on_launch 还原）；None = 清键
        // （写空串——restore 侧 parse::<u32>() 失败即视为无值）
        match v {
            Some(v) => {
                crate::database::dao::settings::set_setting(KEY_WIN_DISK_SAVED, &v.to_string())
            }
            None => crate::database::dao::settings::set_setting(KEY_WIN_DISK_SAVED, ""),
        }
    }
}

// 其他平台（linux 等）：本任务未纳入免休眠 API，exec_* 记日志降级（不 panic）
#[cfg(not(any(target_os = "macos", windows)))]
impl PowerOps for RealOps {
    fn exec_start(&mut self) {
        log::info!("电源保活：当前平台暂不支持，降级跳过");
    }
    fn exec_stop(&mut self) {}
    fn disk_read(&mut self) -> Option<u32> {
        None
    }
    fn disk_set(&mut self, _: Option<u32>) {}
    fn persist_saved(&mut self, _: Option<u32>) {}
}

/// 全局电源锁单例（acquire / release / restore 三入口共享）
static POWER: Lazy<std::sync::Mutex<PowerCore<RealOps>>> =
    Lazy::new(|| std::sync::Mutex::new(PowerCore::new(RealOps)));

/// 远程开启时获取（开关默认开；start_server 真正 spawn 的成功路径调用）
pub fn acquire() {
    if !should_acquire(crate::database::dao::settings::get_setting(KEY_KEEPALIVE)) {
        return;
    }
    POWER.lock().unwrap().acquire();
}

/// 远程关闭 / 应用退出时释放（stop_server 末尾 + lib.rs RunEvent::Exit 调用）
pub fn release() {
    POWER.lock().unwrap().release();
}

/// 崩溃恢复：启动时若 KV 有未还原的磁盘原值 → 写回并清键
/// （mod.rs restore_on_launch 函数体首行调用）
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

// ============================================================
// macOS：caffeinate -ims 子进程管理（CAFFEINATE 全局 Mutex，启动失败 warn 降级不 panic）
// ============================================================

#[cfg(target_os = "macos")]
static CAFFEINATE: Lazy<std::sync::Mutex<Option<std::process::Child>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

#[cfg(target_os = "macos")]
fn real_start_mac() {
    let mut g = CAFFEINATE.lock().unwrap();
    if g.is_some() {
        return;
    }
    match std::process::Command::new("caffeinate")
        .args(["-ims"])
        .spawn()
    {
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

// ============================================================
// Windows：执行状态常驻线程 + 磁盘休眠代设。
// windows crate 0.57 实测签名适配（蓝本形态 → 编译器真实形态）：
//   1. Power* 家族首参为泛型 Param<HKEY>，`None::<&HKEY>` 显式标注走
//      `impl Param<T> for Option<&T>`（即文档要求的 NULL root key）；
//   2. 返回值是 WIN32_ERROR / u32（非 Result），成功 = 0；
//   3. 该族函数受 Win32_System_Registry feature 门控——Cargo.toml 已随
//      Win32_System_Power 一并追加。
// ============================================================

#[cfg(windows)]
mod win {
    use once_cell::sync::Lazy;
    use std::sync::Mutex;
    use windows::core::GUID;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::System::Power::{
        PowerGetActiveScheme, PowerReadACValueIndex, PowerSetActiveScheme, PowerWriteACValueIndex,
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };
    use windows::Win32::System::Registry::HKEY;

    // SUB_DISK / DISKIDLE 官方 GUID（磁盘子组 / 磁盘空闲超时）
    const SUB_DISK: GUID = GUID::from_u128(0x0012ee47_9041_4b5d_9b77_535fba8b1442);
    const DISKIDLE: GUID = GUID::from_u128(0x6738e2c4_e8a5_4a42_b16a_e040e769756e);

    /// 常驻线程停止柄：Some = 保活线程在跑（exec_start 幂等去重）
    static THREAD_STOP: Lazy<Mutex<Option<std::sync::mpsc::Sender<()>>>> =
        Lazy::new(|| Mutex::new(None));

    /// 起常驻线程持有 ES_CONTINUOUS|ES_SYSTEM_REQUIRED 断言（断言绑定线程：随线程存亡）
    pub fn exec_start() {
        let mut g = THREAD_STOP.lock().unwrap();
        if g.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            // ES_CONTINUOUS 断言绑定线程：常驻线程持有，收到 stop 才清除并退出
            let _ = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            let _ = rx.recv();
            // 收尾：ES_CONTINUOUS（不带其他标志）= 清除断言
            let _ = unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        });
        *g = Some(tx);
    }

    /// 停线程：drop Sender → recv Err → 线程清断言退出
    pub fn exec_stop() {
        if let Some(tx) = THREAD_STOP.lock().unwrap().take() {
            drop(tx);
        }
    }

    /// 读当前（AC）磁盘休眠秒数；读失败 → None（跳过磁盘代设）
    pub fn disk_read() -> Option<u32> {
        unsafe {
            let mut scheme: *mut GUID = std::ptr::null_mut();
            if PowerGetActiveScheme(None::<&HKEY>, &mut scheme).0 != 0 {
                return None;
            }
            let mut v = 0u32;
            let code = PowerReadACValueIndex(
                None::<&HKEY>,
                Some(scheme as *const GUID),
                Some(&SUB_DISK),
                Some(&DISKIDLE),
                &mut v,
            );
            free_scheme(scheme);
            (code == 0).then_some(v)
        }
    }

    /// 写（AC）磁盘休眠秒数并激活（PowerSetActiveScheme 生效）
    pub fn disk_set(v: u32) {
        unsafe {
            let mut scheme: *mut GUID = std::ptr::null_mut();
            if PowerGetActiveScheme(None::<&HKEY>, &mut scheme).0 != 0 {
                return;
            }
            let _ = PowerWriteACValueIndex(
                None::<&HKEY>,
                scheme as *const GUID,
                Some(&SUB_DISK),
                Some(&DISKIDLE),
                v,
            );
            let _ = PowerSetActiveScheme(None::<&HKEY>, Some(scheme as *const GUID));
            free_scheme(scheme);
        }
    }

    /// scheme 指针由 API 分配，须 LocalFree 释放（防泄漏）
    unsafe fn free_scheme(scheme: *mut GUID) {
        if !scheme.is_null() {
            let _ = LocalFree(HLOCAL(scheme.cast()));
        }
    }
}

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
        fn exec_start(&mut self) {
            self.started.borrow_mut().push("start");
        }
        fn exec_stop(&mut self) {
            self.started.borrow_mut().push("stop");
        }
        fn disk_read(&mut self) -> Option<u32> {
            self.disk_read
        }
        fn disk_set(&mut self, v: Option<u32>) {
            self.disk_writes.borrow_mut().push(v);
        }
        fn persist_saved(&mut self, v: Option<u32>) {
            self.persisted.borrow_mut().push(v);
        }
    }

    #[test]
    fn acquire_then_release_restores_disk() {
        let mut core = PowerCore::new(FakeOps {
            disk_read: Some(600),
            ..Default::default()
        });
        assert!(core.acquire()); // start + 磁盘 600→0 + 持久化 600
        assert!(core.acquire()); // 幂等：重复 acquire 不再 start
        core.release(); // stop + 磁盘恢复 600 + 持久化清除
        core.release(); // 幂等
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
