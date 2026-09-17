//! 注入消息归一（裁决 6 定死）：手机多行输入 → 字面 `\n` 拼接单行——终端侧永远是
//! 「打字 + 一次回车」，避开 TUI 多次提交与 paste-buffer 粘贴确认两个版本敏感雷区；
//! 桌面不换行展示为已登记的已知限制（二期 spec 附录 C #6）。

/// 换行符（\n 与 \r\n）→ 字面 `\n` 两字符，其余原样
///
/// 注意：首步必须先按 `"\r\n"` 两字符序列整体归一（clippy collapsible_str_replace
/// 建议的单趟 `replace(['\n', '\r'], ..)` 会把 CRLF 拆成两个 `\n`，违背裁决 6 的
/// 「CRLF 整体归一为一个 `\n`」语义，不可采纳）。
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\\n").replace(['\n', '\r'], "\\n")
}

/// 组装最终注入文本：`[mobile <设备花名>] <归一正文>`（W1 来源标记，桌面一眼可辨）
pub fn compose_injection(device_name: &str, text: &str) -> String {
    format!("[mobile {}] {}", device_name, normalize_newlines(text))
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
    }
}
