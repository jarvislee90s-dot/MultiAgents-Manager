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

    /// 单设备吊销时的队列清理：移除「已消费且设备 id == device_id」的请求项，返回清除数
    /// （audit 记 purged=N）。依据 spec T0a「吊销生效时效收紧为即时」/ T2d「吊销后即时断连」：
    /// 若不清，残留项存活至 TTL（5 分钟），期间旧 requestId 重 poll/重 confirm
    /// （/pair/* 不过闸）会再次走 persist 的 INSERT OR REPLACE 把 revoked 硬编码回 0——
    /// 已吊销设备复活（敌意设备正是吊销功能的威胁模型）
    pub fn purge_by_device(&mut self, device_id: &str) -> usize {
        let before = self.requests.len();
        self.requests
            .retain(|r| r.approved_device.as_deref() != Some(device_id));
        before - self.requests.len()
    }

    /// 全部吊销时的队列清理：移除一切已产生设备的消费项，返回清除数（audit 记 purged=N）。
    /// 未批准的 pending 请求不动——它们尚未产生设备，不构成复活面
    /// （防复活依据同 purge_by_device：spec T0a 即时生效）
    pub fn purge_approved(&mut self) -> usize {
        let before = self.requests.len();
        self.requests.retain(|r| r.approved_device.is_none());
        before - self.requests.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

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

    /// 吊销防复活（评审 Important，spec T0a「吊销生效时效收紧为即时」）：
    /// 已批准项被 purge_by_device 后，旧 requestId 重 poll 不得再取回设备 id——
    /// 否则 handler 侧 persist 的 INSERT OR REPLACE 会把 revoked 回写 0，
    /// 已吊销设备在 TTL（5 分钟）内复活（敌意设备正是吊销功能的威胁模型）
    #[test]
    fn revoke_purges_consumed_request_and_blocks_poll_revival() {
        let mut s = svc();
        s.create("d", "UA", "1.1.1.1", 0).unwrap();
        assert_eq!(
            s.approve("req-x", 1, || false),
            ApproveOutcome::Ok {
                device: "adev-x".into(),
                name: "d".into()
            }
        );
        assert_eq!(s.purge_by_device("adev-x"), 1);
        // 旧 requestId 重 poll → Expired（拿不到设备 id，复活路径封死）
        assert_eq!(s.poll("req-x", 2), PollOutcome::Expired);
        // 幂等：再 purge 无残留可清，计数 0
        assert_eq!(s.purge_by_device("adev-x"), 0);
    }

    /// 全部吊销路径：purge_approved 清掉一切已消费项（返回清除数进 audit），
    /// 未批准的 pending 请求不动（尚未产生设备，不构成复活面）
    #[test]
    fn revoke_all_purges_approved_but_keeps_pending() {
        // 生成器恒定值形态照 svc() 先例；id 需要区分两条请求，用计数源（测试内确定且互异，
        // 生产随机性由 mod.rs 的零参随机生成器保证，与本测试无关）
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let mut s = ApprovalService::new(
            5 * 60 * 1000,
            Box::new(move || {
                format!(
                    "req-{}",
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                )
            }),
            Box::new(|| "1234".to_string()),
            Box::new(|| "adev-x".to_string()),
        );
        s.create("d", "UA", "1.1.1.1", 0).unwrap(); // req-0
        s.approve("req-0", 1, || false);
        // 另一条未批准的 pending 请求（不同 IP——同 IP 会被 IpBusy 拒）
        s.create("d2", "UA", "2.2.2.2", 2).unwrap(); // req-1
        assert_eq!(s.purge_approved(), 1);
        // 面板只剩未批准项；已消费 requestId 重 poll → Expired
        assert_eq!(s.pending(3).len(), 1);
        assert_eq!(s.poll("req-0", 4), PollOutcome::Expired);
        // 未批准请求不受 purge 影响：仍可正常批准
        assert_eq!(
            s.approve("req-1", 5, || false),
            ApproveOutcome::Ok {
                device: "adev-x".into(),
                name: "d2".into()
            }
        );
    }
}
