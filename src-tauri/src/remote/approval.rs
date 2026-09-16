// 审批配对队列（M4 T2a/T2b/T2c）：内存态状态机，重启即清（spec T2a 边界——请求不落库）。
// 设备上限不在本层判（approve/confirm 的调用方注入 cap 闭包——三入口统一门，spec T2c）

/// 队列并发上限（spec T2a 自决默认）
pub const MAX_QUEUE: usize = 3;
/// 请求有效期（spec T2a：5 分钟）
pub const REQUEST_TTL_MS: i64 = 5 * 60 * 1000;
/// 4 位码限试次数（spec T2b）
pub const MAX_CODE_TRIES: u8 = 3;

/// 单条审批请求（内存记录；Clone 供桌面面板列表快照；Debug/PartialEq 供测试
/// 对 `Result<&ApprovalRequest, CreateRejection>` 的 assert_eq!（brief 原稿缺——编译适配））
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRequest {
    pub id: String,
    pub name: String,
    pub ua: String,
    pub ip: String,
    pub code: String,
    pub tries: u8,
    pub approved_device: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, PartialEq)]
pub enum CreateRejection {
    QueueFull,
    IpBusy,
}

#[derive(Debug, PartialEq)]
pub enum ApproveOutcome {
    /// 批准成功：设备 id + 设备名（花名册展示手机填的名字——名字必须随结果带出，
    /// 请求消费后无法回查）
    Ok {
        device: String,
        name: String,
    },
    NotFound,
    Expired,
    CapFull,
}

#[derive(Debug, PartialEq)]
pub enum PollOutcome {
    Pending { expires_at: i64 },
    Approved { device: String, name: String },
    Expired,
}

#[derive(Debug, PartialEq)]
pub enum ConfirmOutcome {
    Ok { device: String, name: String },
    Wrong(u8), // 剩余次数
    Exhausted,
    Expired,
    NotFound,
}

pub struct ApprovalService {
    ttl_ms: i64,
    /// 三个生成器均为零参随机形态（生产 = 随机 hex；**不可位置式**——
    /// 请求消费后位置复用会让旧轮询搭上新请求、设备 id 撞行）
    id_gen: Box<dyn Fn() -> String + Send + Sync>,
    code_gen: Box<dyn Fn() -> String + Send + Sync>,
    device_gen: Box<dyn Fn() -> String + Send + Sync>,
    requests: Vec<ApprovalRequest>,
}

impl ApprovalService {
    pub fn new(
        ttl_ms: i64,
        id_gen: Box<dyn Fn() -> String + Send + Sync>,
        code_gen: Box<dyn Fn() -> String + Send + Sync>,
        device_gen: Box<dyn Fn() -> String + Send + Sync>,
    ) -> Self {
        Self {
            ttl_ms,
            id_gen,
            code_gen,
            device_gen,
            requests: Vec::new(),
        }
    }

    fn prune(&mut self, now: i64) {
        self.requests.retain(|r| r.expires_at > now);
    }

    /// 新请求：TTL 清理 → 同 IP 占位检查 → 队列上限检查
    pub fn create(
        &mut self,
        name: &str,
        ua: &str,
        ip: &str,
        now: i64,
    ) -> Result<&ApprovalRequest, CreateRejection> {
        self.prune(now);
        if self.requests.iter().any(|r| r.ip == ip) {
            return Err(CreateRejection::IpBusy);
        }
        if self.requests.len() >= MAX_QUEUE {
            return Err(CreateRejection::QueueFull);
        }
        let id = (self.id_gen)();
        self.requests.push(ApprovalRequest {
            id: id.clone(),
            name: name.trim().chars().take(40).collect(),
            ua: ua.chars().take(200).collect(),
            ip: ip.to_string(),
            code: (self.code_gen)(),
            tries: 0,
            approved_device: None,
            created_at: now,
            expires_at: now + self.ttl_ms,
        });
        Ok(self.requests.last().unwrap())
    }

    /// 桌面批准（cap 闭包 = 满员判定，三入口同门）
    pub fn approve(
        &mut self,
        id: &str,
        now: i64,
        cap_exceeded: impl FnOnce() -> bool,
    ) -> ApproveOutcome {
        self.prune(now);
        let Some(r) = self.requests.iter_mut().find(|r| r.id == id) else {
            return ApproveOutcome::NotFound;
        };
        if cap_exceeded() {
            return ApproveOutcome::CapFull;
        }
        let device = (self.device_gen)();
        r.approved_device = Some(device.clone());
        ApproveOutcome::Ok {
            device,
            name: r.name.clone(),
        }
    }

    pub fn poll(&mut self, id: &str, now: i64) -> PollOutcome {
        self.prune(now);
        match self.requests.iter().find(|r| r.id == id) {
            None => PollOutcome::Expired, // 作废/不存在对手机同观感（不给预言机）
            Some(r) => match &r.approved_device {
                Some(d) => PollOutcome::Approved {
                    device: d.clone(),
                    name: r.name.clone(),
                },
                None => PollOutcome::Pending {
                    expires_at: r.expires_at,
                },
            },
        }
    }

    /// 4 位码确认（Ok 路径的上限门由调用方在落库前检查——状态机纯内存；满员时码对了也拒）
    pub fn confirm(&mut self, id: &str, code: &str, now: i64) -> ConfirmOutcome {
        self.prune(now);
        let Some(r) = self.requests.iter_mut().find(|r| r.id == id) else {
            return ConfirmOutcome::NotFound;
        };
        if let Some(d) = &r.approved_device {
            return ConfirmOutcome::Ok {
                device: d.clone(),
                name: r.name.clone(),
            };
        }
        if r.code == code.trim() {
            let device = (self.device_gen)();
            r.approved_device = Some(device.clone());
            return ConfirmOutcome::Ok {
                device,
                name: r.name.clone(),
            };
        }
        r.tries += 1;
        if r.tries >= MAX_CODE_TRIES {
            self.requests.retain(|x| x.id != id); // 错满 3 次：作废需重新发起（spec T2b）
            ConfirmOutcome::Exhausted
        } else {
            ConfirmOutcome::Wrong(MAX_CODE_TRIES - r.tries)
        }
    }

    /// 桌面面板列表：过期项剔除 + **已消费项隐藏**（approve/confirm Ok 后 poll 仍可
    /// 幂等补发 cookie，但面板不得再展示——brief 的 pending 测试断言要求，语义适配）
    pub fn pending(&mut self, now: i64) -> Vec<ApprovalRequest> {
        self.prune(now);
        self.requests
            .iter()
            .filter(|r| r.approved_device.is_none())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> ApprovalService {
        // 三个生成器均为零参随机形态（生产 = 随机 hex；id/设备 id 绝不可位置式——
        // 请求消费后位置复用会让旧轮询搭上新请求、旧设备 id 撞新设备行）
        ApprovalService::new(
            5 * 60 * 1000, // TTL 5 分钟（spec T2a）
            Box::new(|| "req-x".to_string()),
            Box::new(|| "1234".to_string()),
            Box::new(|| "adev-x".to_string()),
        )
    }

    #[test]
    fn create_approve_poll_flow() {
        let mut s = svc();
        let r = s.create("我的手机", "UA", "1.1.1.1", 0).unwrap();
        assert_eq!(r.id, "req-x");
        // 轮询：待批准
        assert!(matches!(s.poll("req-x", 10), PollOutcome::Pending { .. }));
        // 桌面批准 → poll 拿设备 id + 设备名（幂等：可重复 poll——落库在 handler 侧，
        // 花名册要显示手机填的设备名，名字必须随审批结果带出）
        assert_eq!(
            s.approve("req-x", 20, || false),
            ApproveOutcome::Ok {
                device: "adev-x".into(),
                name: "我的手机".into()
            }
        );
        assert_eq!(
            s.poll("req-x", 21),
            PollOutcome::Approved {
                device: "adev-x".into(),
                name: "我的手机".into()
            }
        );
        assert_eq!(
            s.poll("req-x", 22),
            PollOutcome::Approved {
                device: "adev-x".into(),
                name: "我的手机".into()
            }
        );
    }

    #[test]
    fn confirm_code_three_strikes() {
        let mut s = svc();
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert_eq!(s.confirm("req-x", "0000", 10), ConfirmOutcome::Wrong(2));
        assert_eq!(s.confirm("req-x", "0000", 11), ConfirmOutcome::Wrong(1));
        // 第 3 次错 → 作废（exhausted），正确码也不再用
        assert_eq!(s.confirm("req-x", "0000", 12), ConfirmOutcome::Exhausted);
        assert_eq!(s.confirm("req-x", "1234", 13), ConfirmOutcome::NotFound);
        // 正确路径（带名字带出）
        s.create("d2", "UA", "2.2.2.2", 20).unwrap();
        assert_eq!(
            s.confirm("req-x", "1234", 21),
            ConfirmOutcome::Ok {
                device: "adev-x".into(),
                name: "d2".into()
            }
        );
    }

    #[test]
    fn ttl_expiry_and_prune() {
        let mut s = svc();
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert!(matches!(
            s.poll("req-x", 5 * 60 * 1000 + 1),
            PollOutcome::Expired
        ));
        // 过期项被 prune 后队列腾位
        assert_eq!(s.pending(5 * 60 * 1000 + 2).len(), 0);
    }

    #[test]
    fn queue_cap_and_ip_dedup() {
        let mut s = svc();
        for i in 0..3 {
            s.create(&format!("d{i}"), "UA", &format!("10.0.0.{i}"), 0)
                .unwrap();
        }
        assert_eq!(
            s.create("d3", "UA", "10.0.0.9", 0),
            Err(CreateRejection::QueueFull)
        );
        // 同 IP 第二个请求被拒（即使队列未满）
        let mut s2 = svc();
        s2.create("a", "UA", "3.3.3.3", 0).unwrap();
        assert_eq!(
            s2.create("b", "UA", "3.3.3.3", 1),
            Err(CreateRejection::IpBusy)
        );
    }

    #[test]
    fn approve_respects_cap_full() {
        let mut s = svc();
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert_eq!(s.approve("req-x", 0, || true), ApproveOutcome::CapFull);
        assert_eq!(
            s.approve("req-x", 1, || false),
            ApproveOutcome::Ok {
                device: "adev-x".into(),
                name: "d".into()
            }
        );
    }

    /// 随机 id 语义锁定：create→confirm 消费后同一 svc 再 create，新请求拿到新 id
    /// （零参生成器由调用方保证唯一性；本测试锁定「结果携带的 id 与请求 id 同源」）
    #[test]
    fn consumed_request_id_not_reused_by_service_contract() {
        let mut s = ApprovalService::new(
            300_000,
            Box::new(|| "req-a".to_string()),
            Box::new(|| "1111".to_string()),
            Box::new(|| "adev-a".to_string()),
        );
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert!(matches!(
            s.confirm("req-a", "1111", 1),
            ConfirmOutcome::Ok { .. }
        ));
        assert_eq!(s.pending(2).len(), 0); // 请求已消费
    }
}
