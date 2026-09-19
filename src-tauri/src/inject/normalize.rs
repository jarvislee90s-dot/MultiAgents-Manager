//! 注入消息归一（裁决 6 定死）：手机多行输入 → 字面 `\n` 拼接单行——终端侧永远是
//! 「打字 + 一次回车」，避开 TUI 多次提交与 paste-buffer 粘贴确认两个版本敏感雷区；
//! 桌面不换行展示为已登记的已知限制（二期 spec 附录 C #6）。
//! F7⑩ 追加第二段归一：换行归一之后滤除无文本语义的控制字符（收尾批 P1 定稿为
//! C0 + DEL + C1 三段，依据见 normalize_newlines 注意二）。

/// 换行符（\n 与 \r\n）→ 字面 `\n` 两字符，再滤除 C0 控制字符，其余原样
///
/// 注意一：首步必须先按 `"\r\n"` 两字符序列整体归一（clippy collapsible_str_replace
/// 建议的单趟 `replace(['\n', '\r'], ..)` 会把 CRLF 拆成两个 `\n`，违背裁决 6 的
/// 「CRLF 整体归一为一个 `\n`」语义，不可采纳）。
///
/// 注意二（F7⑩ C0 滤除；收尾批 P1 扩 DEL 与 C1）：换行归一后正文中已无裸 \n/\r
/// （均成字面 `\n` 两字符），此时滤除控制字符即「只删控制字符、不伤换行语义」。依据：
/// - C0（U+0000–U+001F，\t 0x09 一并）：B 族把裸 ESC 当按键吃（M6R F3）；正文
///   控制字符无 TUI 语义，滤除=最安全归一；
/// - DEL（U+007F）：crossterm（B 族 codex）把 0x7F 当退格键——消息含 DEL 直发
///   终端会删除已打入的内容（比忽略更糟），与滤 C0 理由同源；
/// - C1（U+0080–U+009F）：同为无文本语义的不可见控制字符；0x9B/0x9D 在 8-bit
///   模式是 CSI/OSC 引入符，TUI 解析面与 C0 同源；真实用户文本几乎不会有意含
///   C1，粘贴脏字符滤除最安全（NBSP 是 U+00A0 不在 C1 区，不受影响）。
pub fn normalize_newlines(text: &str) -> String {
    let no_newlines = text.replace("\r\n", "\\n").replace(['\n', '\r'], "\\n");
    no_newlines
        .chars()
        .filter(|&c| !is_strippable_control(c))
        .collect()
}

/// 无文本语义控制字符判定（F7⑩ C0；收尾批 P1 扩 DEL 与 C1，依据见
/// [`normalize_newlines`] 注意二）。逐字符（Unicode 标量）判定——配合 `chars()`
/// 过滤天然多字节安全：emoji/中文按整标量整体处理，不存在按字节滤除的撕裂风险
fn is_strippable_control(c: char) -> bool {
    let cp = c as u32;
    cp <= 0x1F || cp == 0x7F || (0x80..=0x9F).contains(&cp)
}

/// 组装最终注入文本：`[mobile <设备花名>] <归一正文>`（W1 来源标记，桌面一眼可辨）
///
/// 设备花名同样归一：昵称来自手机端用户输入，可能含换行，不得破坏单行不变量。
pub fn compose_injection(device_name: &str, text: &str) -> String {
    format!(
        "[mobile {}] {}",
        normalize_newlines(device_name),
        normalize_newlines(text)
    )
}

/// 审计摘要（W5：只存摘要不入全文，防审计库膨胀）
pub fn summarize(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

/// 审计摘要截断长度（W5 只存摘要）
pub const AUDIT_SUMMARY_CHARS: usize = 80;

#[cfg(test)]
mod tests {
    use super::*;

    /// 裁决 6：换行符 → 字面 \n 两字符；其余原样（含已有的字面 \n 不动）
    #[test]
    fn newlines_become_literal_backslash_n() {
        assert_eq!(normalize_newlines("第一行\n第二行"), "第一行\\n第二行");
        assert_eq!(normalize_newlines("已是字面\\n"), "已是字面\\n");
        assert_eq!(normalize_newlines("无换行"), "无换行");
        assert_eq!(normalize_newlines("a\r\nb"), "a\\nb"); // \r\n 整体归一，杜绝裸 \r
    }

    /// W1：[mobile 设备名] 前缀 + 归一正文
    #[test]
    fn compose_prefixes_and_normalizes() {
        assert_eq!(
            compose_injection("iPhone", "改一下\n继续"),
            "[mobile iPhone] 改一下\\n继续"
        );
    }

    /// 审计摘要：超长截断加省略号
    #[test]
    fn summarize_truncates() {
        assert_eq!(summarize("abcdef", 4), "abcd…");
        assert_eq!(summarize("abc", 4), "abc");
        assert_eq!(summarize("一二三四五", 4), "一二三四…"); // 多字节按 chars 计
    }

    /// 裸 CR（奇异客户端）也归一
    #[test]
    fn bare_cr_normalizes() {
        assert_eq!(normalize_newlines("a\rb"), "a\\nb");
    }

    /// 设备花名同样归一（用户可设昵称，堵单行不变量缺口）
    #[test]
    fn compose_normalizes_device_name_too() {
        assert_eq!(
            compose_injection("iPhone\n15", "hi"),
            "[mobile iPhone\\n15] hi"
        );
    }

    /// F7⑩：C0 控制字符滤除——B 族把裸 ESC 当按键吃（M6R F3 实证），正文控制字符
    /// 无 TUI 语义，滤除=最安全归一。字面 `\n` 归一产物与普通中文/ASCII 原样。
    #[test]
    fn c0_controls_are_stripped() {
        // 裸 ESC+CSI、BEL、\t 全部滤除（\t 属 C0）；可见字符原样
        let out = normalize_newlines("a\x1b[31m红\x07色\t缩进");
        assert_eq!(out, "a[31m红色缩进");
        // 零残留断言：产物不含任何 0x00–0x1F 区间字符
        assert!(out.chars().all(|c| (c as u32) > 0x1F));
        // 字面 `\n` 两字符（换行归一产物）保留，同样零 C0
        let with_nl = normalize_newlines("行一\n行二\x1b");
        assert_eq!(with_nl, "行一\\n行二");
        assert!(with_nl.chars().all(|c| (c as u32) > 0x1F));
        // 普通中文/ASCII 原样
        assert_eq!(normalize_newlines("普通中文 abc 123"), "普通中文 abc 123");
        // DEL (0x7F) 不属 C0 区——其滤除依据（B 族退格）与 C0 不同源，单独断言见
        // del_is_stripped（收尾批 P1：便于未来对 DEL/C1 单独回退）
        // 端到端锁：compose_injection 输出同样零 C0（注入通道唯一出口）
        let composed = compose_injection("iPhone", "文本\x1b[0m\x07收尾");
        assert!(composed.chars().all(|c| (c as u32) > 0x1F));
    }

    /// 收尾批 P1：DEL(0x7F) 纳入滤除——crossterm（B 族 codex）把 0x7F 当退格键，
    /// 消息含 DEL 直发终端会删除已打入的内容（比忽略更糟），与滤 C0 理由同源。
    /// 与 C1 分开断言，便于未来单独回退
    #[test]
    fn del_is_stripped() {
        let out = normalize_newlines("del\x7fx");
        assert_eq!(out, "delx", "DEL 必须滤除（B 族退格误删已打内容）");
        assert!(
            !out.chars().any(|c| (c as u32) == 0x7F),
            "零残留：产物不含 DEL"
        );
        // 端到端锁：compose_injection 同样零 DEL（注入通道唯一出口）
        let composed = compose_injection("iPhone", "a\u{7f}b");
        assert!(!composed.chars().any(|c| (c as u32) == 0x7F));
    }

    /// 收尾批 P1 C1 连带评估（默认裁决：一并滤除）：C1（U+0080–U+009F）同为无文本
    /// 语义的不可见控制字符；0x9B/0x9D 在 8-bit 模式是 CSI/OSC 引入符，TUI 解析面
    /// 与 C0 同源；真实用户文本几乎不会有意含 C1，粘贴脏字符滤除最安全。与 DEL
    /// 分开断言，便于未来单独回退
    #[test]
    fn c1_controls_are_stripped() {
        // 采样：0x9B（8-bit CSI 引入符）、0x85（NEL）、0x90（DCS）、0x9D（OSC 引入符）
        let out = normalize_newlines("a\u{9b}31m\u{85}b\u{90}c\u{9d}d");
        assert_eq!(out, "a31mbcd", "C1 控制字符必须滤除");
        assert!(
            !out.chars().any(|c| (0x80..=0x9F).contains(&(c as u32))),
            "零残留：产物不含任何 C1 字符"
        );
        // NBSP（U+00A0）不在 C1 区——合法展示字符不受连坐，原样保留
        assert_eq!(normalize_newlines("a\u{a0}b"), "a\u{a0}b");
    }

    /// 收尾批 P2：多字节安全钉死——混入控制字符的正文经逐字符（Unicode 标量）
    /// 过滤，emoji（😀 U+1F600，4 字节）与中文必须完整保留、控制字符零残留；
    /// 防未来误改成按字节滤除（会把多字节标量撕成无效字节）
    #[test]
    fn emoji_multibyte_survives_filter() {
        let out = normalize_newlines("部署😀\u{7f}完成\u{1b}[31m中文\u{85}收尾");
        assert_eq!(out, "部署😀完成[31m中文收尾");
        // emoji 原样（整标量存活，未撕裂）
        assert!(out.contains('\u{1F600}'), "emoji 必须原样保留：{out}");
        assert!(
            out.contains("部署") && out.contains("中文"),
            "中文必须原样保留"
        );
        // 控制字符零残留（C0 + DEL + C1 全区间）
        assert!(
            out.chars().all(|c| {
                let cp = c as u32;
                cp > 0x1F && cp != 0x7F && !(0x80..=0x9F).contains(&cp)
            }),
            "零残留：产物不含 C0/DEL/C1 任何字符"
        );
    }
}
