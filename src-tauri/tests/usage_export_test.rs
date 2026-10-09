//! Task 21 **fix round 1（评审裁决 A）**：**生产入口链条**的行为锁。
//!
//! 为什么必须有本文件：任务书 Step 1 的 lib 用例只锁「目录注入版落盘核」（`save_*_file_in`）
//! 与 `exports_dir()` 的**形状**——`save_text_file` / `save_bytes_file` 与两条
//! `#[tauri::command]`（真正的生产路径）当时**零调用**。评审给出的存活变异：
//! ① 把 `export_save_text` 改调 `save_bytes_file`（类型合法、可编译）→ 生产路径**丢 BOM**，
//! 13 条 lib 用例**全绿**；② 把 `save_text_file` 的目录由 `exports_dir()` 换成 `/tmp` → 同样全绿。
//! 而本任务第一个可观测事实正是「写进导出目录」。**2026-10-07 裁决 A2 后该目录 = 系统「下载」文件夹**
//! （原 `~/.tuvis/exports/`；本文件随之把期望值从 `<home>/.tuvis/exports` 改为 `<home>/Downloads`）。
//!
//! 本文件用 `support::setup()` 把 `HOME` / `TUVIS_HOME` 指到 tempdir
//! （**绝不写用户真实下载目录**，与 GC 19 同一条纪律），再调**生产入口**，
//! 断言的是**磁盘事实**（路径形状 / 绝对性 / 真存在 / BOM 三字节 / 字节数 / 逐字节相等）。
//!
//! 顺带锁上 **reveal 覆盖退让的一半**：白名单根含**导出目录本身**（A2 后 = 下载目录；
//! 见 `resource.rs::reveal_allowed_roots()`），`setup()` 重定向后
//! 落盘路径**存在** ⇒ `ensure_reveal_allowed` 可判定（不再依赖开发机真实 `~/.tuvis` 状态）。
//! 仍**不**断言 `reveal_dir` 的真实系统打开（那属 Task 24 的人工目验）。
//!
//! 隔离说明：本文件不触发采集（零 `DSH_HOME` / `KIMI_CODE_HOME` 暴露），也不需要串行锁——
//! `support::setup()` 是 `Once`（`call_once` 会阻塞其它线程直到环境就绪），而各用例只写
//! **互不相同**的文件名，彼此无共享可变状态。
mod support;

use multi_agents_manager_lib::commands::export::{
    export_save_bytes, export_save_text, MAX_BINARY_BYTES,
};
use std::path::PathBuf;

/// 生产导出目录 `exports_dir()` 的形状：`<home>/Downloads`（A2 后的系统下载目录；原 `.tuvis/exports`）。
/// `support::setup()` 把 `HOME` 与 `TUVIS_HOME` 同时指到同一个 tempdir；生产代码在 debug/test
/// 构建下认 `TUVIS_HOME`（Windows 的 `dirs::home_dir()` 会忽略 `HOME`，故必须两者都看）。
fn expected_exports_dir() -> PathBuf {
    let home = std::env::var_os("TUVIS_HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .expect("support::setup() 之后家目录必须可得");
    home.join("Downloads")
}

/// 文本生产入口：`export_save_text` → 落 `<home>/Downloads/<name>`、返回**绝对路径**、
/// 磁盘内容是 **BOM + 入参**。
/// 存活变异①（改调 `save_bytes_file`）→ 第 3 行字节断言必红；存活变异②（目录换 `/tmp`）→ 父目录断言必红。
#[test]
fn production_text_entry_writes_bom_file_into_downloads() {
    support::setup();
    let dir = expected_exports_dir();
    let name = "fix-r1-text.csv";
    let content = "groupKey,label\nclaude,claude\n";
    let target = dir.join(name);
    assert!(
        !target.exists(),
        "前提：目标文件此刻不得存在（否则下面的落盘断言不成立）"
    );

    let path =
        export_save_text(name.to_string(), content.to_string()).expect("生产入口必须落盘成功");
    let p = std::path::Path::new(&path);

    assert!(p.is_absolute(), "契约 §3：返回**绝对路径**，实际 {path}");
    assert_eq!(
        p.parent(),
        Some(dir.as_path()),
        "必须落进 <home>/Downloads（生产目录，A2），实际 {path}"
    );
    assert!(dir.is_dir(), "导出目录必须被自动创建");
    assert!(p.exists(), "文件必须真的落盘（不是只返回了字符串）");
    let raw = std::fs::read(p).expect("读回磁盘字节");
    assert_eq!(
        raw.len(),
        content.len() + 3,
        "磁盘字节数必须 == BOM(3) + 内容（生产入口丢 BOM 时这里先红）"
    );
    assert_eq!(
        &raw[..3],
        &[0xEF, 0xBB, 0xBF],
        "生产入口的 .csv 必须前置 UTF-8 BOM"
    );
    assert_eq!(&raw[3..], content.as_bytes(), "BOM 之后逐字节等于入参");
}

/// 二进制生产入口：`export_save_bytes` → 绝对路径 + 落进同目录 + **逐字节等于解码结果且不加 BOM**。
/// 入参用真实 canvas 形态 `data:image/png;base64,…`，顺带在生产链条上验一次前缀容忍。
#[test]
fn production_bytes_entry_writes_exact_bytes_into_downloads() {
    support::setup();
    let dir = expected_exports_dir();
    let name = "fix-r1-bytes.png";
    let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let target = dir.join(name);
    assert!(!target.exists(), "前提：目标文件此刻不得存在");

    let path = export_save_bytes(
        name.to_string(),
        "data:image/png;base64,iVBORw0KGgo=".to_string(),
    )
    .expect("生产入口必须落盘成功");
    let p = std::path::Path::new(&path);

    assert!(p.is_absolute(), "契约 §3：返回**绝对路径**，实际 {path}");
    assert_eq!(
        p.parent(),
        Some(dir.as_path()),
        "必须落进 <home>/Downloads（A2），实际 {path}"
    );
    assert!(p.exists(), "文件必须真的落盘");
    let raw = std::fs::read(p).expect("读回磁盘字节");
    assert_eq!(
        raw.len(),
        png.len(),
        "磁盘字节数必须等于解码后长度（多 3 字节即是被误加 BOM）"
    );
    assert_eq!(raw, png, "二进制落盘必须逐字节等于解码结果、不得前置 BOM");
}

/// **reveal 覆盖退让的一半**（fix round 1 新增的锁）：`setup()` 重定向 `HOME` 后，
/// 两条生产入口的落盘路径都能过既有 `ensure_reveal_allowed`（白名单根 `home_dir()/.tuvis`）。
/// 另一半（`reveal_dir` 真的把目录在系统文件管理器里打开）仍需 **Task 24 人工目验**。
///
/// **Windows 上不可判定**：`dirs::home_dir()` 在 Windows 忽略 `HOME`（见 `tests/support.rs` 的注释），
/// 而 `ensure_reveal_allowed` 固定读 `dirs::home_dir()` → 测试无法把白名单根重定向到 tempdir
/// （会变成断言开发机真实家目录状态）。故本用例 `#[cfg(unix)]`。
#[cfg(unix)]
#[test]
fn production_entries_pass_existing_reveal_whitelist() {
    use multi_agents_manager_lib::commands::resource::ensure_reveal_allowed;
    support::setup();
    let tname = "fix-r1-reveal.csv";
    let tpath = export_save_text(tname.to_string(), "a,b\n".to_string()).unwrap();
    let tcanon = match ensure_reveal_allowed(&tpath) {
        Ok(p) => p,
        Err(e) => panic!("文本入口落盘路径必须能过 reveal 白名单：{e}"),
    };
    assert!(
        tcanon.ends_with(tname),
        "canonicalize 后仍指向同一文件：{}",
        tcanon.display()
    );

    let bname = "fix-r1-reveal.png";
    let bpath = export_save_bytes(bname.to_string(), "iVBORw0KGgo=".to_string()).unwrap();
    let bcanon = match ensure_reveal_allowed(&bpath) {
        Ok(p) => p,
        Err(e) => panic!("二进制入口落盘路径必须能过 reveal 白名单：{e}"),
    };
    assert!(
        bcanon.ends_with(bname),
        "canonicalize 后仍指向同一文件：{}",
        bcanon.display()
    );
}

/// **裁决 B**（fix round 1）：base64 是 4/3 放大，超限入参必须在 **decode 之前**按**载荷长度**
/// 拒绝（否则 400 MB 串先吃 ~300 MB 解码缓冲）。
/// 本用例的判别点：错误文案必须能区分「预先按 base64 长度拒绝」与「解码后被写手按解码长度拒绝」
/// ——去掉预判后这里会拿到 `导出文件过大（… 字节 > … 上限）`（无 `base64` 字样）→ **真红**。
/// 阈值取 `MAX_BINARY_BYTES / 3 * 4 + 4` 之外 4 个字符（≈22 MB 入参，不会真去分配 300 MB）。
#[test]
fn production_bytes_entry_rejects_oversize_base64_before_decoding() {
    support::setup();
    let dir = expected_exports_dir();
    let name = "fix-r1-huge.png";
    let over = MAX_BINARY_BYTES / 3 * 4 + 4 + 4;
    let err = export_save_bytes(name.to_string(), "A".repeat(over)).unwrap_err();
    assert!(
        err.contains("base64"),
        "必须在 decode **之前**按 base64 载荷长度拒绝（文案须可判别是预判）：{err}"
    );
    assert!(err.contains("过大"), "仍须是「过大」这一类错误：{err}");
    assert!(!dir.join(name).exists(), "超限被拒时不得留下任何文件");
}
