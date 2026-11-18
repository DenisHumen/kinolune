// SPDX-License-Identifier: MPL-2.0
//! NET-010 done-when: a "Windows share" (Samba with wsdd) and a Jellyfin
//! server in Docker are found on the Docker bridge
//! (`tests/lan/run.sh`). Needs `PA_LAN_IFACE` (the bridge address of the
//! host), `PA_LAN_SMB_IP` and `PA_LAN_JELLYFIN_IP`.
#![cfg(feature = "server-tests")]

use std::net::Ipv4Addr;
use std::time::Duration;

use pa_lan_probe::{LanService, Options, ServiceKind, discover};

fn env_ip(name: &str) -> Ipv4Addr {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: run tests/lan/run.sh"))
        .parse()
        .unwrap()
}

fn search(hosts: Vec<Ipv4Addr>) -> Vec<LanService> {
    // Probes may be dropped right after a container starts: a few tries.
    let mut last = Vec::new();
    for _ in 0..3 {
        last = discover(&Options {
            wait: Duration::from_secs(3),
            interfaces: vec![env_ip("PA_LAN_IFACE")],
            hosts: hosts.clone(),
        })
        .unwrap();
        let smb = last
            .iter()
            .any(|s| s.kind == ServiceKind::Smb && s.host == env_ip("PA_LAN_SMB_IP"));
        let jf = last
            .iter()
            .any(|s| s.kind == ServiceKind::Jellyfin && s.host == env_ip("PA_LAN_JELLYFIN_IP"));
        if smb && jf {
            break;
        }
    }
    last
}

fn find(found: &[LanService], kind: ServiceKind, ip: Ipv4Addr) -> LanService {
    found
        .iter()
        .find(|s| s.kind == kind && s.host == ip)
        .cloned()
        .unwrap_or_else(|| panic!("{kind:?} on {ip} not found in {found:?}"))
}

/// Broadcast and multicast only (the bridge is a /16: no sweep), as on a
/// home network: WS-Discovery finds the computer, the metadata names it.
#[test]
fn finds_the_windows_share_and_jellyfin_by_broadcast() {
    let found = search(Vec::new());
    let smb = find(&found, ServiceKind::Smb, env_ip("PA_LAN_SMB_IP"));
    assert_eq!(smb.name, "KINO-WINPC");
    assert_eq!(smb.workgroup.as_deref(), Some("KINOTEST"));
    assert_eq!(smb.url, format!("smb://{}/", env_ip("PA_LAN_SMB_IP")));
    assert!(smb.id.as_deref().is_some_and(|i| i.starts_with("urn:uuid:")));

    let jf = find(&found, ServiceKind::Jellyfin, env_ip("PA_LAN_JELLYFIN_IP"));
    assert_eq!(jf.url, format!("http://{}:8096", env_ip("PA_LAN_JELLYFIN_IP")));
    assert!(jf.id.is_some());
    assert!(!jf.name.is_empty());
    // Jellyfin does not answer the Emby question: listed once.
    assert!(
        !found
            .iter()
            .any(|s| s.kind == ServiceKind::Emby && s.host == jf.host)
    );
}

/// The unicast NetBIOS node status (as the sweep of a small network
/// sends it) names the computer on its own, and a search that also
/// probes the host by unicast still lists it once.
#[test]
fn netbios_node_status_names_the_share() {
    use pa_lan_probe::{netbios, udp};
    let ip = env_ip("PA_LAN_SMB_IP");
    let ifaces = udp::interfaces(&[env_ip("PA_LAN_IFACE")]);
    let answers = udp::exchange(
        &ifaces,
        |_| vec![(netbios::nbns(ip), netbios::node_status_request(0x4242))],
        Duration::from_secs(2),
    )
    .unwrap();
    let fs = answers
        .iter()
        .filter(|a| *a.from.ip() == ip)
        .find_map(|a| netbios::parse_node_status(&a.data, 0x4242))
        .as_deref()
        .and_then(netbios::file_server)
        .expect("node status of the share");
    assert_eq!(fs.name, "KINO-WINPC");
    assert_eq!(fs.workgroup.as_deref(), Some("KINOTEST"));

    let found = search(vec![ip]);
    assert_eq!(
        found
            .iter()
            .filter(|s| s.kind == ServiceKind::Smb && s.host == ip)
            .count(),
        1
    );
}
