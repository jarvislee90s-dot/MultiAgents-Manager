// M5 实锤定通道（2026-09-18 用户裁决的「实锤」项）：隧道流量全部由 MAM 拉起的
// cloudflared 进程从回环转发进来——反查系统 TCP 连接表，找到持有「来连套接字」
// 的进程 PID，与各通道子进程 PID 账本（tunnel::owned_channel_pids）对账，
// 通道归属即为实锤，不依赖域名名单。查不到（本机浏览器直连 / 表不可得 / macOS）
// 返回 None → 调用方回落既有 Host 判定（gate 豁免 / classify_via）。
//
// OS 覆盖：Windows（`windows` crate GetExtendedTcpTable，枚举含 owning PID）
// 与 Linux（/proc/net/tcp 解析 + /proc/<pid>/fd inode 映射）为一等实现；
// macOS 无 procfs，libproc FFI 本期不做 → 空表回落。
//
// 纯函数内核与 OS 采集分离（ConnRow 注入），内核全量单测；OS 采集只做薄壳。

use std::net::IpAddr;

/// 连接表一行（采集层产出；PID = 持有该套接字的进程）
#[derive(Debug, Clone)]
pub(crate) struct ConnRow {
    pub local_ip: IpAddr,
    pub local_port: u16,
    /// remote 两字段保留：与系统连接表行结构同构（后续过滤规则细化时可用）
    #[allow(dead_code)]
    pub remote_ip: IpAddr,
    #[allow(dead_code)]
    pub remote_port: u16,
    pub pid: u32,
}

/// 实锤内核（纯函数）：从连接表里找「回环上 local_port == 来连临时端口」且
/// PID 在通道账本中的套接字 → 该 PID 的通道。来连临时端口当刻唯一，双重过滤
/// （端口 + PID ∈ MAM 账本）后误配为天文数字级；查不到 → None（调用方回落）
pub(crate) fn tunnel_channel_for_conn_with(
    rows: &[ConnRow],
    conn_remote_port: u16,
    owned: &[(u32, &'static str)],
) -> Option<&'static str> {
    let row = rows.iter().find(|r| {
        r.local_ip.is_loopback()
            && r.local_port == conn_remote_port
            && owned.iter().any(|(p, _)| *p == r.pid)
    })?;
    owned
        .iter()
        .find(|(p, _)| *p == row.pid)
        .map(|(_, kind)| *kind)
}

/// 生产入口：采集真实连接表 → 内核对账。
/// 只在「来源为回环」时被调用（gate/pair_pin 侧先做 is_loopback 廉价判断），
/// 非回环流量不付连接表枚举开销
pub(crate) fn tunnel_channel_for_conn(conn_remote_port: u16) -> Option<&'static str> {
    let owned = super::tunnel::owned_channel_pids();
    if owned.is_empty() {
        return None; // 无隧道连接器在跑：零开销短路
    }
    let rows = os_rows();
    tunnel_channel_for_conn_with(&rows, conn_remote_port, &owned)
}

/// OS 连接表采集（只保留回环行，缩减后续匹配面）
#[allow(unused_variables)]
fn os_rows() -> Vec<ConnRow> {
    #[cfg(target_os = "windows")]
    {
        windows_rows()
    }
    #[cfg(target_os = "linux")]
    {
        linux_rows()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Vec::new() // macOS 等：无 procfs，libproc FFI 本期不做 → 回落 Host 判定
    }
}

/// Windows：GetExtendedTcpTable（TCP_TABLE_OWNER_PID_ALL，行内自带 owning PID）。
/// 端口字段为网络字节序（big-endian），须从 u32 高低字节换回 u16
#[cfg(target_os = "windows")]
fn windows_rows() -> Vec<ConnRow> {
    use std::mem::size_of;
    use windows::Win32::NetworkManagement::IpHelper::GetExtendedTcpTable;
    use windows::Win32::NetworkManagement::IpHelper::TCP_TABLE_OWNER_PID_ALL;
    use windows::Win32::Networking::WinSock::AF_INET;

    unsafe {
        // 首查取所需尺寸，再按尺寸分配二查（标准两段式）
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let ret = GetExtendedTcpTable(
            Some(buf.as_mut_ptr() as _),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
        if ret != 0 {
            return Vec::new();
        }
        let count = *(buf.as_ptr() as *const u32) as usize;
        // MIB_TCPROW_OWNER_PID：State(u32) LocalAddr(u32) LocalPort(u32) RemoteAddr(u32)
        // RemotePort(u32) OwningPid(u32) —— 紧跟在 dwNumEntries 之后
        let base = buf.as_ptr() as usize + size_of::<u32>();
        let row_size = size_of::<u32>() * 6;
        let mut out = Vec::new();
        for i in 0..count {
            let p = base + i * row_size;
            // 外层 unsafe 块已覆盖原始指针解引用，闭包内无需再标
            let read_u32 = |off: usize| u32::from_le_bytes(*((p + off) as *const [u8; 4]));
            let local_addr = read_u32(size_of::<u32>());
            let local_port_raw = read_u32(size_of::<u32>() * 2);
            let remote_addr = read_u32(size_of::<u32>() * 3);
            let remote_port_raw = read_u32(size_of::<u32>() * 4);
            let pid = read_u32(size_of::<u32>() * 5);
            // 地址为网络字节序存于 u32 低位；端口高 16 位网络序
            let ip = IpAddr::from([
                (local_addr & 0xff) as u8,
                ((local_addr >> 8) & 0xff) as u8,
                ((local_addr >> 16) & 0xff) as u8,
                ((local_addr >> 24) & 0xff) as u8,
            ]);
            if !ip.is_loopback() {
                continue; // 只保留回环行（缩减匹配面）
            }
            let local_port =
                ((local_port_raw >> 8) & 0xff) as u16 | ((local_port_raw & 0xff) << 8) as u16;
            let remote_port =
                ((remote_port_raw >> 8) & 0xff) as u16 | ((remote_port_raw & 0xff) << 8) as u16;
            let rip = IpAddr::from([
                (remote_addr & 0xff) as u8,
                ((remote_addr >> 8) & 0xff) as u8,
                ((remote_addr >> 16) & 0xff) as u8,
                ((remote_addr >> 24) & 0xff) as u8,
            ]);
            out.push(ConnRow {
                local_ip: ip,
                local_port,
                remote_ip: rip,
                remote_port,
                pid,
            });
        }
        out
    }
}

/// Linux：/proc/net/tcp 解析（回环行）+ /proc/<pid>/fd 的 socket:[inode] 映射。
/// MAM 与 cloudflared 同用户运行，fd 可见性满足
#[cfg(target_os = "linux")]
fn linux_rows() -> Vec<ConnRow> {
    fn parse_hex_ip_port(s: &str) -> Option<(IpAddr, u16)> {
        let (ip_hex, port_hex) = s.split_once(':')?;
        let port = u16::from_str_radix(port_hex, 16).ok()?;
        let raw = u32::from_str_radix(ip_hex, 16).ok()?;
        // /proc 里地址为小端主机序的十六进制（字节序已被读出，逐字节重组）
        let ip = IpAddr::from([
            (raw & 0xff) as u8,
            ((raw >> 8) & 0xff) as u8,
            ((raw >> 16) & 0xff) as u8,
            ((raw >> 24) & 0xff) as u8,
        ]);
        Some((ip, port))
    }
    let text = std::fs::read_to_string("/proc/net/tcp").unwrap_or_default();
    // inode → pid 映射（一次全量扫描）
    let mut inode_pid: std::collections::HashMap<u64, u32> = Default::default();
    let Ok(proc_iter) = std::fs::read_dir("/proc") else {
        return Vec::new(); // /proc 不可读（非 Linux/容器受限）→ 空表回落 Host 判定
    };
    for entry in proc_iter.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(&fd_dir) else {
            continue; // 无权限/进程已退出 → 跳过
        };
        for fd in fds.flatten() {
            let Ok(link) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let link = link.to_string_lossy();
            if let Some(rest) = link.strip_prefix("socket:[") {
                // CI 教训（2026-09-18）：本块 cfg(target_os=linux) 在 Windows 本机不编译，
                // clippy 错误只有 Linux CI 可见——`Some(x.ok())` 应直接 `if let Ok`
                if let Ok(ino) = rest.trim_end_matches(']').parse::<u64>() {
                    inode_pid.insert(ino, pid);
                }
            }
        }
    }
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 {
            continue;
        }
        let Some((lip, lport)) = parse_hex_ip_port(cols[1]) else {
            continue;
        };
        let Some((rip, rport)) = parse_hex_ip_port(cols[2]) else {
            continue;
        };
        if !lip.is_loopback() {
            continue;
        }
        let Ok(inode) = cols[9].parse::<u64>() else {
            continue;
        };
        let Some(pid) = inode_pid.get(&inode).copied() else {
            continue;
        };
        out.push(ConnRow {
            local_ip: lip,
            local_port: lport,
            remote_ip: rip,
            remote_port: rport,
            pid,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn lb() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }
    fn row(local_port: u16, remote_port: u16, pid: u32) -> ConnRow {
        ConnRow {
            local_ip: lb(),
            local_port,
            remote_ip: lb(),
            remote_port,
            pid,
        }
    }

    const OWNED: &[(u32, &str)] = &[(111, "quick"), (222, "named")];

    /// 实锤正例：来连临时端口 51000 的套接字归属 PID 111 → quick
    #[test]
    fn maps_conn_to_quick_by_pid() {
        let rows = vec![row(51000, 9420, 111), row(52000, 9420, 999)];
        assert_eq!(
            tunnel_channel_for_conn_with(&rows, 51000, OWNED),
            Some("quick")
        );
    }

    /// named 通道同理；未知 PID（编外连接器/本机浏览器）不匹配 → None（回落 Host 判定）
    #[test]
    fn maps_named_and_rejects_unknown_pid() {
        let rows = vec![row(53000, 9420, 222), row(54000, 9420, 999)];
        assert_eq!(
            tunnel_channel_for_conn_with(&rows, 53000, OWNED),
            Some("named")
        );
        assert_eq!(tunnel_channel_for_conn_with(&rows, 54000, OWNED), None);
        assert_eq!(tunnel_channel_for_conn_with(&rows, 59999, OWNED), None);
    }

    /// 端口为唯一键：来连端口对不上任何行 → None；非回环行被采集层忽略（语义锚定）
    #[test]
    fn no_match_when_port_absent() {
        let rows = vec![row(51000, 9420, 111)];
        assert_eq!(tunnel_channel_for_conn_with(&rows, 51001, OWNED), None);
        assert!(rows.iter().all(|r| r.local_ip.is_loopback()));
    }
}
