//! 注入路由表（W3，纯核）：会话属性 → 通道决策 + 可见性预期。路由按宿主形态与
//! 工具门判定，**工具无关的终端宿主一律优先终端注入**（无头对已开 TUI 会分叉，
//! spec 裁决「路由规则按宿主形态，工具无关」）。M11 无头落地后此处仅追加分支。

use crate::session::model::ProcessForm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Tmux,
    Iterm2,
    TerminalApp,
    WindowsConsole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Realtime,
    AfterRefresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    Injectable {
        candidates: Vec<Channel>,
        visibility: Visibility,
    },
    NotInjectable {
        reason_code: &'static str,
        reason: String,
    },
}

/// 黑盒/另评工具（宪法附 A 矩阵）：WorkBuddy ❌、dsh ◐ 另评、OpenClaw ◐ gateway、
/// ZCode 固定走无头（D6 在册限定，M11）
fn tool_gate(tool: &str) -> Option<(&'static str, String)> {
    match tool {
        "workbuddy" => Some(("blackbox", "WorkBuddy 黑盒，无外部写 API".into())),
        "dsh" => Some(("blackbox", "dsh 写通道另评（web API/插件生态）".into())),
        "openclaw" => Some(("blackbox", "OpenClaw 走 gateway，另评".into())),
        "zcode" => Some(("headless_only", "ZCode 走无头通道（M11）".into())),
        _ => None,
    }
}

pub fn route(agent_tool_id: &str, form: ProcessForm, pid: u32, platform: &str) -> RouteOutcome {
    if pid == 0 {
        return RouteOutcome::NotInjectable {
            reason_code: "no_process",
            reason: "会话进程不存在".into(),
        };
    }
    if let Some((code, reason)) = tool_gate(agent_tool_id) {
        return RouteOutcome::NotInjectable {
            reason_code: code,
            reason,
        };
    }
    if form == ProcessForm::App {
        return RouteOutcome::NotInjectable {
            reason_code: "app_form",
            reason: "APP 形态无外部写 API（computer-use 属应急预案 D7）".into(),
        };
    }
    match platform {
        "macos" => RouteOutcome::Injectable {
            candidates: vec![Channel::Tmux, Channel::Iterm2, Channel::TerminalApp],
            visibility: Visibility::Realtime,
        },
        "windows" => RouteOutcome::Injectable {
            candidates: vec![Channel::WindowsConsole],
            visibility: Visibility::Realtime,
        },
        _ => RouteOutcome::NotInjectable {
            reason_code: "platform",
            reason: "当前平台不支持注入".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> ProcessForm {
        ProcessForm::Cli
    }
    fn app() -> ProcessForm {
        ProcessForm::App
    }

    /// APP 形态不可注入（WorkBuddy/Codex APP 等——computer-use 唯一通路属 D7 应急级）
    #[test]
    fn app_form_not_injectable() {
        let r = route("claude", app(), 100, "macos");
        assert!(matches!(
            r,
            RouteOutcome::NotInjectable {
                reason_code: "app_form",
                ..
            }
        ));
    }

    /// WorkBuddy 黑盒 / dsh 另评 / ZCode 走无头（M11）/ OpenClaw gateway 另评
    #[test]
    fn tool_gates() {
        for (tool, code) in [
            ("workbuddy", "blackbox"),
            ("dsh", "blackbox"),
            ("zcode", "headless_only"),
            ("openclaw", "blackbox"),
        ] {
            let r = route(tool, cli(), 100, "macos");
            assert!(
                matches!(r, RouteOutcome::NotInjectable { reason_code, .. } if reason_code == code),
                "{tool}"
            );
        }
    }

    /// pid=0（未读卡兜底/进程不在）不可注入
    #[test]
    fn dead_process_not_injectable() {
        assert!(matches!(
            route("claude", cli(), 0, "macos"),
            RouteOutcome::NotInjectable {
                reason_code: "no_process",
                ..
            }
        ));
    }

    /// macOS 可注入家（claude/codex/opencode/kimi CLI）：三通道候选按由简到繁（裁决 14）
    #[test]
    fn macos_candidates_ordered() {
        let r = route("kimi", cli(), 42, "macos");
        let RouteOutcome::Injectable {
            candidates,
            visibility,
        } = r
        else {
            panic!()
        };
        assert_eq!(
            candidates,
            vec![Channel::Tmux, Channel::Iterm2, Channel::TerminalApp]
        );
        assert!(matches!(visibility, Visibility::Realtime));
    }

    /// Windows：单通道候选（宿主可达性由 M6 结论与引擎层处理）
    #[test]
    fn windows_single_channel() {
        assert!(matches!(route("claude", cli(), 42, "windows"),
            RouteOutcome::Injectable { candidates, .. } if candidates == vec![Channel::WindowsConsole]));
    }

    /// 其他平台不支持
    #[test]
    fn linux_not_supported() {
        assert!(matches!(
            route("claude", cli(), 42, "linux"),
            RouteOutcome::NotInjectable {
                reason_code: "platform",
                ..
            }
        ));
    }
}
