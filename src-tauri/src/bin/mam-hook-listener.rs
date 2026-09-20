//! mam-hook-listener — 原生 hook 事件监听 helper（批次甲 T1 · issue #74 根因 2）
//!
//! 使命：替代 hooks.rs 生成的 bash 版 status-hook.sh——三家 CLI 的 hook 脚本现为
//! bash，在 codex 原生 shell（cmd 包装+清环境）下裸 `bash` 不可解析，钩子从未运行
//! （「Hook failed exit 1」根因）。本 helper 是 Rust 编译的零 shell 依赖原生进程，
//! hook 配置直接指向本 exe 绝对路径（claude `command` / codex 官方 `commandWindows`
//! 字段，注册与迁移见 `monitor::hooks`）。
//!
//! # 实现红线（任务书 §1，违反即返工；内核侧单测在 hook_listener 模块文末）
//!
//! 1. **监听模式统一 exit 0 + 空 stdout**——codex 侧 exit 2+stderr = Deny（劫持
//!    审批）、JSON stdout = 劫持审批框（与 claude 语义相反，claude 的 exit 2 不
//!    生效）；只有「0 + 空输出」= decline to decide → 审批流原样继续。任何输入
//!    （非法 JSON / 空 stdin / 非 UTF-8 / 写盘失败 / 未预期 panic）都不得产生
//!    stdout 字节与非零退出码。本 bin 用「静默 panic hook + catch_unwind + 全路径
//!    吞错」三重保证；stderr 保守起见同样完全静默（helper 无 log 初始化）。
//! 2. **瞬时完成**——codex 命令钩子默认 600s 超时、Interrupt/SessionEnd 仅 1s：
//!    读 stdin → serde 解析 → 同目录临时文件+rename 原子写 → 退出，毫秒级；
//!    无网络、无重试、无等待。
//! 3. `async:true` 钩子被 codex 跳过——本 helper 天然同步快速，注册侧（hooks.rs）
//!    不得带 async（T2 落实）。
//! 4. payload 差异（claude/codex/kimi 的 tool_input/message 等）是 T2/T3 的事——
//!    本 helper 只做「stdin JSON → session_id/hook_event_name → 事件文件」薄管道。
//!
//! 逻辑本体在共享内核 [`hook_listener`]（`#[path]` 引入同一源文件，lib 侧
//! `monitor::hook_listener` 同源可测；独立编译单元不链接整个 lib——mam-marker
//! 先例，保证体积小、启动毫秒级）。bin 主体 = 读 stdin → 内核 → 写文件 → exit 0。
//!
//! 分发：应用启动时由 `monitor::hooks::ensure_hook_script` 把与主程序同目录的本
//! exe 拷到 `~/.mam/bin/`（mam-marker 同一管道，无条件覆盖保证升级生效）；helper
//! 未构建/未随包分发是合法状态——注册侧检测不到即回落 bash 形态（零回归）。
//!
//! 构建：本 bin 挂 `required-features = ["hook-listener"]` 门（Cargo.toml；
//! marker-helper 蕴含本 feature，release.yml / CI 已显式携带 marker-helper，
//! 发行与门禁自动构建，macOS universal 打包不受影响——mam-marker 同款门控理由）。
//! 事件目录经 `hook_listener::default_events_dir()`（MAM_HOME debug 重定向同
//! connection.rs 先例），测试一律 tempdir 直注（零接触真实 ~/.mam）。

#[path = "../monitor/hook_listener.rs"]
mod hook_listener;

use std::io::Read;

fn main() {
    // 红线 1 双保险：panic hook 静默（默认 panic 输出走 stderr 且 exit 101，同样
    // 污染钩子通道）+ catch_unwind 压平为正常返回（隐式 exit 0）。release profile
    // 未开 panic=abort，unwind 可用
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(run_main);
}

/// 进程主流程：读 stdin → 内核解析 → 原子写事件文件 → 返回（调用方隐式 exit 0）。
/// 全路径不打印、不以非零码退出
fn run_main() {
    let mut input = String::new();
    // 读失败（管道关闭/非法 UTF-8）静默吞掉：红线 1 优先于一切诊断
    let _ = std::io::stdin().read_to_string(&mut input);
    run(&input, &hook_listener::default_events_dir());
}

/// 主流程内核（tempdir 可测缝）：解析失败/白名单不过 → 不落盘；写失败 → 吞掉。
/// 任何输入不 panic、不输出（bin 侧单测断言）
fn run(input: &str, events_dir: &std::path::Path) {
    if let Some(parsed) = hook_listener::parse_hook_stdin(input) {
        let ts = hook_listener::now_unix();
        let _ = hook_listener::write_event_file(events_dir, &parsed, ts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 红线 1 bin 面：一切非法输入零落盘零输出（不 panic、不写任何文件）
    #[test]
    fn run_is_silent_noop_on_invalid_input() {
        let tmp = tempfile::tempdir().unwrap();
        let cases = [
            "",                                                          // 空 stdin
            "not json at all",                                           // 非 JSON
            "{\"session_id\":",                                          // 截断 JSON
            "{\"hook_event_name\":\"Stop\"}",                            // 缺 session_id
            "{\"session_id\":\"sid-1\"}",                                // 缺 event_name
            "{\"session_id\":123,\"hook_event_name\":\"Stop\"}",         // 类型非法
            "{\"session_id\":\"../evil\",\"hook_event_name\":\"Stop\"}", // 路径注入
        ];
        for bad in cases {
            run(bad, tmp.path());
        }
        assert_eq!(
            std::fs::read_dir(tmp.path()).unwrap().count(),
            0,
            "非法输入不得产生任何文件"
        );
    }

    /// 合法 payload（claude 形态样本）→ 事件文件按读取侧格式落盘
    #[test]
    fn run_writes_event_for_valid_payload() {
        let tmp = tempfile::tempdir().unwrap();
        run(
            r#"{"session_id":"01a08083-5ca0","hook_event_name":"Stop","cwd":"E:\\proj"}"#,
            tmp.path(),
        );
        let body = std::fs::read_to_string(tmp.path().join("01a08083-5ca0.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["event"], "Stop");
        assert_eq!(v["session_id"], "01a08083-5ca0");
        assert_eq!(v["cwd"], "E:\\proj");
        assert!(v["ts"].is_i64(), "ts 必须是 unix 秒整数");
        assert!(v["last_event_at"].as_str().unwrap().ends_with('Z'));
    }
}
