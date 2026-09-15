// SKILL.md frontmatter 专属声明解析（spec §6，M3 Task 17）
//
// 识别键（宽容别名，顶层标量/数组字段）：supported_agents / agents /
// compatible_tools / tools；值支持 YAML flow 数组 `["a", "b"]` 或逗号串
// `"a, b"`。键缺失 / 坏 YAML / 无 frontmatter / 空列表 → None（视为未声明，
// 「自动识别只建议不强制，真值永远是 resource_bindings 手动值」）。
// 手写轻量解析，不引入 serde_yaml 依赖（brief 约束）。

/// frontmatter 命中后的专属预填建议（serde camelCase，与前端
/// `FrontmatterSuggestion` 同构）。导入路径与存量扫描命令共用。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontmatterSuggestion {
    pub extension_id: String,
    pub tools: Vec<String>,
}

/// 识别键别名（宽容别名，spec §6）。逐行扫描首个命中的键生效
const SUPPORTED_AGENT_KEYS: [&str; 4] = ["supported_agents", "agents", "compatible_tools", "tools"];

/// 解析 SKILL.md frontmatter 的专属声明，命中返回工具 id 列表。
/// 规则：仅识别首个 `---` 围栏块（必须从字节 0 开始、必须闭合）内的
/// 顶层键（列 0 起，缩进行的嵌套键不误配）；解析失败一律 None。
pub fn parse_supported_agents(skill_md: &str) -> Option<Vec<String>> {
    // 围栏必须从字节 0 开始：首行恰为 "---"（str::lines 已把 \r\n 折算为 \n）
    let mut lines = skill_md.lines();
    if lines.next()? != "---" {
        return None;
    }
    // 收集块内行直到闭合围栏；未闭合 → 坏 frontmatter
    let mut block: Vec<&str> = Vec::new();
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        block.push(line);
    }
    if !closed {
        return None;
    }
    // 逐行找首个命中的顶层别名键；缩进的嵌套键因不满足「列 0 起 + 冒号」天然排除。
    // 首个命中即定生死：值解析失败（坏 YAML）按 spec 视为未声明
    for line in block {
        for key in SUPPORTED_AGENT_KEYS {
            if let Some(after) = line.strip_prefix(key).and_then(|a| a.strip_prefix(':')) {
                return parse_value(after.trim());
            }
        }
    }
    None
}

/// 值解析：flow 数组 `[a, b]` 或逗号串 `"a, b"`（裸标量宽容接受为单项）。
/// 空值（裸键/块列表不在支持范围）、坏形状、空结果 → None
fn parse_value(value: &str) -> Option<Vec<String>> {
    if value.is_empty() {
        // `tools:` 下一行 `- a` 的块列表不在「顶层标量/数组」支持范围 → 未声明
        return None;
    }
    let raw_items: Vec<&str> = if let Some(inner) = value.strip_prefix('[') {
        // flow 数组必须闭合，否则坏 YAML
        inner.strip_suffix(']')?.split(',').collect()
    } else if value.starts_with('{') {
        // flow map 非标量/数组 → 坏 YAML
        return None;
    } else {
        value.split(',').collect()
    };
    let tools: Vec<String> = raw_items
        .iter()
        .filter_map(|item| {
            let tool = strip_quotes(item.trim());
            if tool.is_empty() {
                None // 空项丢弃
            } else {
                Some(tool)
            }
        })
        .collect();
    if tools.is_empty() {
        None // 空列表 = 未声明
    } else {
        Some(tools)
    }
}

/// 剥掉项两侧的单/双引号（成对 trim，混合引号串宽容处理）
fn strip_quotes(item: &str) -> String {
    item.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// 建议判定核心（三条路径共用）：frontmatter 命中且绑定表无该行 → Some。
/// 绑定集合由调用方提供——存量扫描一次拉全表逐技能判定，避免逐项查库。
/// 「绑定表已有行（含空列表）= 用户已裁决」→ 不再建议（手动为真值源，spec §6）
pub fn suggestion_for_content(
    name: &str,
    content: &str,
    bound_ids: &std::collections::HashSet<String>,
) -> Option<FrontmatterSuggestion> {
    let tools = parse_supported_agents(content)?;
    let extension_id = format!("skill-{}", name);
    if bound_ids.contains(&extension_id) {
        return None;
    }
    Some(FrontmatterSuggestion {
        extension_id,
        tools,
    })
}

/// 安装/导入成功后的即时建议（两条手动导入路径共用）：读 SSOT 仓库中该技能的
/// SKILL.md 判定。读取失败（非技能目录/IO 错误）→ None，绝不阻断导入本身
pub fn suggest_for_skill(name: &str) -> Option<FrontmatterSuggestion> {
    let skill_md = crate::linker::ensure_repo_dir().join(name).join("SKILL.md");
    let content = std::fs::read_to_string(skill_md).ok()?;
    let bound_ids: std::collections::HashSet<String> = crate::database::list_resource_bindings()
        .into_iter()
        .map(|b| b.extension_id)
        .collect();
    suggestion_for_content(name, &content, &bound_ids)
}

#[cfg(test)]
mod tests {
    use super::parse_supported_agents;

    fn fm(body: &str) -> String {
        format!("---\n{}---\n正文", body)
    }

    /// 标准数组：supported_agents: ["a", "b"]
    #[test]
    fn standard_flow_array() {
        let md = fm("name: x\ndescription: d\nsupported_agents: [\"claude\", \"codex\"]\n");
        assert_eq!(
            parse_supported_agents(&md),
            Some(vec!["claude".to_string(), "codex".to_string()])
        );
    }

    /// 逗号串：quoted scalar 内逗号切分
    #[test]
    fn quoted_comma_string() {
        let md = fm("supported_agents: \"claude, codex\"\n");
        assert_eq!(
            parse_supported_agents(&md),
            Some(vec!["claude".to_string(), "codex".to_string()])
        );
    }

    /// 四别名各一：agents / compatible_tools / tools（supported_agents 见上）
    #[test]
    fn alias_keys_all_recognized() {
        for key in ["agents", "compatible_tools", "tools"] {
            let md = fm(&format!("{}: [\"claude\"]\n", key));
            assert_eq!(
                parse_supported_agents(&md),
                Some(vec!["claude".to_string()]),
                "别名 {key} 应被识别"
            );
        }
    }

    /// 坏 YAML：flow 数组未闭合 → None
    #[test]
    fn bad_yaml_unclosed_bracket() {
        let md = fm("tools: [claude, codex\n");
        assert_eq!(parse_supported_agents(&md), None);
    }

    /// 坏 YAML：flow map（非标量/数组）→ None
    #[test]
    fn bad_yaml_flow_map() {
        let md = fm("tools: {a: b}\n");
        assert_eq!(parse_supported_agents(&md), None);
    }

    /// 缺键：frontmatter 存在但只有其他键 → None
    #[test]
    fn missing_key_returns_none() {
        let md = fm("name: x\ndescription: 只有两个键\nversion: 1\n");
        assert_eq!(parse_supported_agents(&md), None);
    }

    /// 无 frontmatter：正文不以 `---` 开头 → None
    #[test]
    fn no_frontmatter_returns_none() {
        assert_eq!(parse_supported_agents("# 标题\n正文"), None);
    }

    /// `---` 不在字节 0（前置空行/正文）→ 无 frontmatter
    #[test]
    fn delimiter_not_at_byte_zero() {
        assert_eq!(
            parse_supported_agents("\n---\ntools: [claude]\n---\n"),
            None
        );
    }

    /// `---` 块未闭合 → None
    #[test]
    fn unterminated_block() {
        let md = "---\ntools: [claude]\n".to_string();
        assert_eq!(parse_supported_agents(&md), None);
    }

    /// 嵌套/缩进键不误配：`  tools:` 列 0 不起 → 视为缺键
    #[test]
    fn nested_indented_key_not_matched() {
        let md = fm("name: x\n  tools: [claude]\n");
        assert_eq!(parse_supported_agents(&md), None);
    }

    /// 键名前缀不误配：`tools_extra:` 不是 `tools:`
    #[test]
    fn key_prefix_not_matched() {
        let md = fm("tools_extra: [claude]\n");
        assert_eq!(parse_supported_agents(&md), None);
    }

    /// 引号剥离：数组项单/双引号混用均剥掉
    #[test]
    fn quotes_stripped() {
        let md = fm("tools: [\"claude\", 'codex', opencode]\n");
        assert_eq!(
            parse_supported_agents(&md),
            Some(vec![
                "claude".to_string(),
                "codex".to_string(),
                "opencode".to_string()
            ])
        );
    }

    /// 空列表 / 全空项 → None（空 = 未声明）
    #[test]
    fn empty_list_returns_none() {
        assert_eq!(parse_supported_agents(&fm("tools: []\n")), None);
        assert_eq!(parse_supported_agents(&fm("tools: [\"\", ]\n")), None);
    }

    /// 空项丢弃 + 空白容忍：[a, , b] → [a, b]
    #[test]
    fn empty_items_dropped() {
        let md = fm("tools: [claude, , codex]\n");
        assert_eq!(
            parse_supported_agents(&md),
            Some(vec!["claude".to_string(), "codex".to_string()])
        );
    }

    /// CRLF 行尾容忍（Windows 编辑器产物）
    #[test]
    fn crlf_tolerated() {
        let md = "---\r\ntools: [\"claude\", \"codex\"]\r\n---\r\n正文\r\n";
        assert_eq!(
            parse_supported_agents(md),
            Some(vec!["claude".to_string(), "codex".to_string()])
        );
    }

    /// 裸标量（无引号无逗号）宽容接受为单项；块列表（`tools:` 下一行 `- a`）
    /// 不在支持范围 → None
    #[test]
    fn bare_scalar_single_item_block_list_unsupported() {
        assert_eq!(
            parse_supported_agents(&fm("tools: codex\n")),
            Some(vec!["codex".to_string()])
        );
        assert_eq!(parse_supported_agents(&fm("tools:\n  - claude\n")), None);
    }
}
