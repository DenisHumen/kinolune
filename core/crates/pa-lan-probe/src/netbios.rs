// SPDX-License-Identifier: MPL-2.0
//! NetBIOS name service (RFC 1001/1002, UDP 137): the node status
//! (`NBSTAT`) of a host lists its names — a unique name with the suffix
//! `0x20` is a file server, a group name with `0x00` is its workgroup.
//! The name query of `__MSBROWSE__` finds the master browsers of a
//! network, which then get a node status of their own.

use std::net::{Ipv4Addr, SocketAddrV4};

/// The NetBIOS name service port.
pub const NBNS_PORT: u16 = 137;

/// The file server service suffix.
const FILE_SERVER: u8 = 0x20;
/// The workstation (and workgroup) suffix.
const WORKSTATION: u8 = 0x00;
/// Node status records list at most this many names (one byte count).
const MAX_NAMES: usize = 255;

/// First-level encoding of a 16-byte NetBIOS name (RFC 1001 §14.1).
fn encode_name(name: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(34);
    out.push(32);
    for b in name {
        out.push(b'A' + (b >> 4));
        out.push(b'A' + (b & 0x0f));
    }
    out.push(0);
    out
}

fn header(id: u16, flags: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(50);
    v.extend_from_slice(&id.to_be_bytes());
    v.extend_from_slice(&flags.to_be_bytes());
    // One question, no records.
    v.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    v
}

/// A node status request for the wildcard name `*`.
#[must_use]
pub fn node_status_request(id: u16) -> Vec<u8> {
    let mut name = [0u8; 16];
    name[0] = b'*';
    let mut v = header(id, 0);
    v.extend(encode_name(&name));
    // NBSTAT, IN.
    v.extend_from_slice(&[0x00, 0x21, 0x00, 0x01]);
    v
}

/// A broadcast name query for `\x01\x02__MSBROWSE__\x02\x01`.
#[must_use]
pub fn master_browser_query(id: u16) -> Vec<u8> {
    let mut name = [0u8; 16];
    name.copy_from_slice(b"\x01\x02__MSBROWSE__\x02\x01");
    // Recursion desired + broadcast.
    let mut v = header(id, 0x0110);
    v.extend(encode_name(&name));
    // NB, IN.
    v.extend_from_slice(&[0x00, 0x20, 0x00, 0x01]);
    v
}

/// The NBNS address of `host`.
#[must_use]
pub fn nbns(host: Ipv4Addr) -> SocketAddrV4 {
    SocketAddrV4::new(host, NBNS_PORT)
}

/// One name of a node status answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NbName {
    /// The name without padding.
    pub name: String,
    /// The suffix (16th byte).
    pub suffix: u8,
    /// A group name (workgroup, domain) rather than a unique one.
    pub group: bool,
}

/// A host that runs a file server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileServer {
    /// Its NetBIOS name (`KINO-WINPC`).
    pub name: String,
    /// Its workgroup or domain, if it lists one.
    pub workgroup: Option<String>,
}

/// Checks a positive answer to `id` and returns the position after the
/// header and the answer name.
fn answer_start(d: &[u8], id: u16) -> Option<usize> {
    if d.len() < 12 || u16::from_be_bytes([d[0], d[1]]) != id {
        return None;
    }
    let flags = u16::from_be_bytes([d[2], d[3]]);
    // Response, rcode 0, at least one answer.
    if flags & 0x8000 == 0 || flags & 0x000f != 0 {
        return None;
    }
    if u16::from_be_bytes([d[6], d[7]]) == 0 {
        return None;
    }
    // The answer name: labels up to a zero length, or a pointer.
    let mut i = 12;
    loop {
        let len = *d.get(i)?;
        if len & 0xc0 == 0xc0 {
            i += 2;
            break;
        }
        i += 1;
        if len == 0 {
            break;
        }
        i += usize::from(len);
        if i > d.len() {
            return None;
        }
    }
    Some(i)
}

/// Whether `d` is a positive answer to the master browser query `id`.
#[must_use]
pub fn is_master_browser_answer(d: &[u8], id: u16) -> bool {
    answer_start(d, id).is_some()
}

/// Reads the names of a node status answer to `id`.
#[must_use]
pub fn parse_node_status(d: &[u8], id: u16) -> Option<Vec<NbName>> {
    let i = answer_start(d, id)?;
    // Type, class, TTL, RDLENGTH.
    let rr = d.get(i..i + 10)?;
    if u16::from_be_bytes([rr[0], rr[1]]) != 0x21 {
        return None;
    }
    let rdlen = usize::from(u16::from_be_bytes([rr[8], rr[9]]));
    let rdata = d.get(i + 10..(i + 10 + rdlen).min(d.len()))?;
    let count = usize::from(*rdata.first()?).min(MAX_NAMES);
    let mut names = Vec::with_capacity(count);
    for n in 0..count {
        let Some(entry) = rdata.get(1 + n * 18..1 + (n + 1) * 18) else {
            break;
        };
        let raw = &entry[..15];
        let suffix = entry[15];
        let flags = u16::from_be_bytes([entry[16], entry[17]]);
        let name: String = raw
            .iter()
            .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
            .collect::<String>()
            .trim()
            .to_owned();
        if name.is_empty() {
            continue;
        }
        names.push(NbName {
            name,
            suffix,
            group: flags & 0x8000 != 0,
        });
    }
    Some(names)
}

/// The file server in a node status: its unique `0x20` name and its
/// group `0x00` name; `None` when it shares no files.
#[must_use]
pub fn file_server(names: &[NbName]) -> Option<FileServer> {
    let name = names
        .iter()
        .find(|n| n.suffix == FILE_SERVER && !n.group)?
        .name
        .clone();
    let workgroup = names
        .iter()
        .find(|n| n.suffix == WORKSTATION && n.group)
        .map(|n| n.name.clone());
    Some(FileServer { name, workgroup })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Samba node status answer (captured in the NET-010 LAN test).
    const SAMBA: &[u8] = b"\x124\x84\x00\x00\x00\x00\x01\x00\x00\x00\x00 CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\x00\x00!\x00\x01\x00\x00\x00\x00\x00\xad\x07KINO-WINPC     \x00\x04\x00KINO-WINPC     \x03\x04\x00KINO-WINPC      \x04\x00\x01\x02__MSBROWSE__\x02\x01\x84\x00KINOTEST       \x00\x84\x00KINOTEST       \x1d\x04\x00KINOTEST       \x1e\x84\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";

    #[test]
    fn reads_a_samba_node_status() {
        let names = parse_node_status(SAMBA, 0x1234).unwrap();
        assert_eq!(names.len(), 7);
        assert_eq!(
            file_server(&names),
            Some(FileServer {
                name: "KINO-WINPC".into(),
                workgroup: Some("KINOTEST".into())
            })
        );
        // Another transaction, a request, a truncated answer.
        assert!(parse_node_status(SAMBA, 0x1235).is_none());
        assert!(parse_node_status(&node_status_request(0x1234), 0x1234).is_none());
        for cut in [0, 11, 40, 56, 70] {
            let _ = parse_node_status(&SAMBA[..cut], 0x1234);
        }
        assert!(parse_node_status(&SAMBA[..50], 0x1234).is_none());
    }

    #[test]
    fn workstations_without_a_file_server_are_not_shares() {
        let names = vec![
            NbName {
                name: "LAPTOP".into(),
                suffix: 0,
                group: false,
            },
            NbName {
                name: "WORKGROUP".into(),
                suffix: 0,
                group: true,
            },
        ];
        assert!(file_server(&names).is_none());
    }

    #[test]
    fn reads_a_master_browser_answer() {
        let ok = b"\x125\x85\x80\x00\x00\x00\x01\x00\x00\x00\x00 ABACFPFPENFDECFCEPFHFDEFFPFPACAB\x00\x00 \x00\x01\x00\x03\xf4\x80\x00\x06\x80\x00\xac\x11\x00\x0b";
        assert!(is_master_browser_answer(ok, 0x1235));
        assert!(!is_master_browser_answer(ok, 0x1236));
        let mut negative = ok.to_vec();
        negative[3] = 0x83;
        assert!(!is_master_browser_answer(&negative, 0x1235));
        // The query encodes the name as the answer does.
        assert_eq!(&master_browser_query(0x1235)[12..46], &ok[12..46]);
        assert_eq!(node_status_request(0x1234)[12..46], SAMBA[12..46]);
    }

    #[test]
    fn hostile_answers_do_not_panic() {
        // A count of names larger than the data, a huge RDLENGTH.
        let mut d = SAMBA[..57].to_vec();
        d[56] = 0xff;
        d[54] = 0xff;
        d[55] = 0xff;
        let names = parse_node_status(&d, 0x1234).unwrap();
        assert!(names.is_empty());
        let mut loopy = SAMBA[..12].to_vec();
        loopy.extend_from_slice(&[0x3f; 4]);
        assert!(parse_node_status(&loopy, 0x1234).is_none());
    }
}
