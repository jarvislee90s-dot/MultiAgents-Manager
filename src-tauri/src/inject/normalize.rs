//! 注入消息归一（裁决 6 定死）：手机多行输入 → 字面 `\n` 拼接单行——终端侧永远是
//! 「打字 + 一次回车」，避开 TUI 多次提交与 paste-buffer 粘贴确认两个版本敏感雷区；
//! 桌面不换行展示为已登记的已知限制（二期 spec 附录 C #6）。
//! F7⑩ 追加第二段归一：换行归一之后滤除 C0 控制字符（依据见 normalize_newlines）。

/// 换行符（\n 与 \r\n）→ 字面 `\n` 两字符，再滤除 C0 控制字符，其余原样
///
/// 注意一：首步必须先按 `"\r\n"` 两字符序列整体归一（clippy collapsible_str_replace
/// 建议的单趟 `replace(['\n', '\r'], ..)` 会把 CRLF 拆成两个 `\n`，违背裁决 6 的
/// 「CRLF 整体归一为一个 `\n`」语义，不可采纳）。
///
/// 注意二（F7⑩ C0 滤除）：换行归一后正文中已无裸 \n/\r（均成字面 `\n` 两字符），
/// 此时把 C0（U+0000–U+001F）全部滤除即「只删控制字符、不伤换行语义」。依据：
/// B 族把裸 ESC 当按键吃（M6R F3）；正文控制字符无 TUI 语义，滤除=最安全归一。
/// \t (0x09) 属 C0 一并移除；0x7F（DEL）不属 C0，不动。
pub fn normalize_newlines(text: &str) -> String {
    let no_newlines = text.replace("\r\n", "\\n").replace(['\n', '\r'], "\\n");
    no_newlines.chars().filter(|&c| !is_c0(c)).collect()
}

/// C0 控制字符判定：U+0000–U+001F（0x7F DEL 不属 C0，保留）
fn is_c0(c: char) -> bool {
    (c as u32) <= 0x1F
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
        // DEL (0x7F) 不属 C0，不动
        assert_eq!(normalize_newlines("del\x7fx"), "del\u{7f}x");
        // 端到端锁：compose_injection 输出同样零 C0（注入通道唯一出口）
        let composed = compose_injection("iPhone", "文本\x1b[0m\x07收尾");
        assert!(composed.chars().all(|c| (c as u32) > 0x1F));
    }
}
