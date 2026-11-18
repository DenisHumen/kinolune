// SPDX-License-Identifier: MPL-2.0
//! WS-Discovery (2005/04, as Windows uses it for "Network"): a `Probe` to
//! `239.255.255.250:3702`, `ProbeMatches` from computers
//! (`pub:Computer`), `Resolve` for those that send no `XAddrs` (wsdd), and
//! the metadata `Get` (WS-Transfer) over HTTP for the computer name and
//! workgroup.

use std::net::{Ipv4Addr, SocketAddrV4};

use url::Url;

use crate::xml::{self, El, escape};

/// The WS-Discovery multicast group and port.
pub const WSD_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 3702);

/// Largest metadata answer read over HTTP.
pub const MAX_METADATA_BYTES: u64 = 64 * 1024;

/// Most matches read from one datagram.
const MAX_MATCHES: usize = 16;

const NS: &str = r#"xmlns:soap="http://www.w3.org/2003/05/soap-envelope" xmlns:wsa="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:wsd="http://schemas.xmlsoap.org/ws/2005/04/discovery" xmlns:wsdp="http://schemas.xmlsoap.org/ws/2006/02/devprof" xmlns:pub="http://schemas.microsoft.com/windows/pub/2005/07""#;

fn envelope(to: &str, action: &str, message_id: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><soap:Envelope {NS}><soap:Header><wsa:To>{}</wsa:To><wsa:Action>{action}</wsa:Action><wsa:MessageID>{}</wsa:MessageID><wsa:ReplyTo><wsa:Address>http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous</wsa:Address></wsa:ReplyTo></soap:Header><soap:Body>{body}</soap:Body></soap:Envelope>"#,
        escape(to),
        escape(message_id),
    )
}

/// A `Probe` for computers.
#[must_use]
pub fn probe_message(message_id: &str) -> String {
    envelope(
        "urn:schemas-xmlsoap-org:ws:2005:04:discovery",
        "http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe",
        message_id,
        "<wsd:Probe><wsd:Types>wsdp:Device pub:Computer</wsd:Types></wsd:Probe>",
    )
}

/// A `Resolve` of `endpoint` (its `EndpointReference` address).
#[must_use]
pub fn resolve_message(message_id: &str, endpoint: &str) -> String {
    envelope(
        "urn:schemas-xmlsoap-org:ws:2005:04:discovery",
        "http://schemas.xmlsoap.org/ws/2005/04/discovery/Resolve",
        message_id,
        &format!(
            "<wsd:Resolve><wsa:EndpointReference><wsa:Address>{}</wsa:Address></wsa:EndpointReference></wsd:Resolve>",
            escape(endpoint)
        ),
    )
}

/// The metadata `Get` of `endpoint`.
#[must_use]
pub fn get_message(message_id: &str, endpoint: &str) -> String {
    envelope(
        endpoint,
        "http://schemas.xmlsoap.org/ws/2004/09/transfer/Get",
        message_id,
        "",
    )
}

/// One computer that answered a probe or a resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsdMatch {
    /// Its endpoint address (`urn:uuid:…`), stable across restarts.
    pub endpoint: String,
    /// Where its metadata answers: only `http(s)` URLs on the address
    /// that sent the match.
    pub xaddrs: Vec<Url>,
}

/// Reads `ProbeMatches` or `ResolveMatches` sent by `from` in answer to
/// `message_id`. Only computers (`pub:Computer`) are kept — printers and
/// scanners answer probes too — and only `XAddrs` on `from` itself.
#[must_use]
pub fn parse_matches(datagram: &[u8], from: Ipv4Addr, message_id: &str) -> Vec<WsdMatch> {
    let Some(root) = std::str::from_utf8(datagram).ok().and_then(xml::parse) else {
        return Vec::new();
    };
    if root.text_of("RelatesTo") != Some(message_id) {
        return Vec::new();
    }
    let action = root.text_of("Action").unwrap_or("");
    let resolve = action.ends_with("/ResolveMatches");
    if !resolve && !action.ends_with("/ProbeMatches") {
        return Vec::new();
    }
    let mut found = Vec::new();
    root.find_all(
        if resolve { "ResolveMatch" } else { "ProbeMatch" },
        &mut found,
    );
    found
        .into_iter()
        .take(MAX_MATCHES)
        .filter_map(|m| read_match(m, from, resolve))
        .collect()
}

fn read_match(m: &El, from: Ipv4Addr, resolve: bool) -> Option<WsdMatch> {
    let endpoint = m.find("EndpointReference")?.text_of("Address")?;
    if endpoint.len() > 256 {
        return None;
    }
    let types = m.text_of("Types").unwrap_or("");
    let computer = types
        .split_whitespace()
        .any(|t| t.rsplit(':').next() == Some("Computer"));
    // A resolve answers for the endpoint we asked about: it may leave
    // the types out.
    if !computer && !(resolve && types.is_empty()) {
        return None;
    }
    let xaddrs = m
        .text_of("XAddrs")
        .unwrap_or("")
        .split_whitespace()
        .take(8)
        .filter_map(|x| Url::parse(x).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .filter(|u| {
            u.host_str()
                .and_then(|h| h.parse::<Ipv4Addr>().ok())
                .is_some_and(|h| h == from)
        })
        .collect();
    Some(WsdMatch {
        endpoint: endpoint.to_owned(),
        xaddrs,
    })
}

/// What a computer says about itself in its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerInfo {
    /// Its name (`KINO-WINPC`).
    pub name: String,
    /// Its workgroup or domain, if it says.
    pub workgroup: Option<String>,
}

/// Reads the metadata answer of a `Get`: `pub:Computer` is
/// `NAME/Workgroup:WG`, `NAME/Domain:D` or `NAME/NotJoined`; without it,
/// the `FriendlyName` of the device.
#[must_use]
pub fn parse_metadata(text: &str) -> Option<ComputerInfo> {
    let root = xml::parse(text)?;
    if let Some(c) = root.text_of("Computer") {
        let (name, rest) = c.split_once('/').unwrap_or((c, ""));
        let workgroup = rest
            .split_once(':')
            .map(|(_, w)| w.trim())
            .filter(|w| !w.is_empty())
            .map(str::to_owned);
        let name = name.trim();
        if !name.is_empty() {
            return Some(ComputerInfo {
                name: name.to_owned(),
                workgroup,
            });
        }
    }
    root.text_of("FriendlyName").map(|n| ComputerInfo {
        name: n.to_owned(),
        workgroup: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "urn:uuid:ca03f8ad-2219-4ffe-aaa5-bc876db429c7";

    fn matches_xml(action: &str, relates: &str, inner: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope" xmlns:wsa="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:wsd="http://schemas.xmlsoap.org/ws/2005/04/discovery" xmlns:wsdp="http://schemas.xmlsoap.org/ws/2006/02/devprof" xmlns:pub="http://schemas.microsoft.com/windows/pub/2005/07"><soap:Header><wsa:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/{action}</wsa:Action><wsa:RelatesTo>{relates}</wsa:RelatesTo></soap:Header><soap:Body>{inner}</soap:Body></soap:Envelope>"#
        )
    }

    #[test]
    fn reads_wsdd_probe_matches_and_drops_printers() {
        // As wsdd 0.8 answers (captured in the NET-010 LAN test): no XAddrs.
        let xml = matches_xml(
            "ProbeMatches",
            ID,
            "<wsd:ProbeMatches><wsd:ProbeMatch><wsa:EndpointReference><wsa:Address>urn:uuid:3c30b6e0-c4ba-5ce4-a53a-8fd700da53b3</wsa:Address></wsa:EndpointReference><wsd:Types>wsdp:Device pub:Computer</wsd:Types><wsd:MetadataVersion>1</wsd:MetadataVersion></wsd:ProbeMatch>\
             <wsd:ProbeMatch><wsa:EndpointReference><wsa:Address>urn:uuid:printer</wsa:Address></wsa:EndpointReference><wsd:Types>wsdp:Device wprt:PrintDeviceType</wsd:Types></wsd:ProbeMatch></wsd:ProbeMatches>",
        );
        let from = Ipv4Addr::new(192, 168, 1, 7);
        let m = parse_matches(xml.as_bytes(), from, ID);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].endpoint, "urn:uuid:3c30b6e0-c4ba-5ce4-a53a-8fd700da53b3");
        assert!(m[0].xaddrs.is_empty());
        // An answer to someone else's probe, or not a match at all.
        assert!(parse_matches(xml.as_bytes(), from, "urn:uuid:other").is_empty());
        let hello = matches_xml("Hello", ID, "");
        assert!(parse_matches(hello.as_bytes(), from, ID).is_empty());
        assert!(parse_matches(b"\xff\xfe", from, ID).is_empty());
        assert!(parse_matches(b"<a>", from, ID).is_empty());
    }

    #[test]
    fn keeps_xaddrs_on_the_sender_only() {
        let xml = matches_xml(
            "ResolveMatches",
            ID,
            "<wsd:ResolveMatches><wsd:ResolveMatch><wsa:EndpointReference><wsa:Address>urn:uuid:a</wsa:Address></wsa:EndpointReference><wsd:Types>wsdp:Device pub:Computer</wsd:Types><wsd:XAddrs>http://192.168.1.7:5357/a http://203.0.113.1/x ftp://192.168.1.7/ http://[fe80::1]:5357/a</wsd:XAddrs></wsd:ResolveMatch></wsd:ResolveMatches>",
        );
        let m = parse_matches(xml.as_bytes(), Ipv4Addr::new(192, 168, 1, 7), ID);
        assert_eq!(m.len(), 1);
        let x: Vec<&str> = m[0].xaddrs.iter().map(Url::as_str).collect();
        assert_eq!(x, ["http://192.168.1.7:5357/a"]);
    }

    #[test]
    fn reads_the_computer_name_and_workgroup() {
        let meta = r#"<soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope" xmlns:wsx="http://schemas.xmlsoap.org/ws/2004/09/mex" xmlns:wsdp="http://schemas.xmlsoap.org/ws/2006/02/devprof" xmlns:pub="http://schemas.microsoft.com/windows/pub/2005/07"><soap:Body><wsx:Metadata><wsx:MetadataSection Dialect="http://schemas.xmlsoap.org/ws/2006/02/devprof/ThisDevice"><wsdp:ThisDevice><wsdp:FriendlyName>WSD Device KINO-WINPC</wsdp:FriendlyName></wsdp:ThisDevice></wsx:MetadataSection><wsx:MetadataSection Dialect="http://schemas.xmlsoap.org/ws/2006/02/devprof/Relationship"><wsdp:Relationship><wsdp:Host><pub:Computer>KINO-WINPC/Workgroup:KINOTEST</pub:Computer></wsdp:Host></wsdp:Relationship></wsx:MetadataSection></wsx:Metadata></soap:Body></soap:Envelope>"#;
        assert_eq!(
            parse_metadata(meta),
            Some(ComputerInfo {
                name: "KINO-WINPC".into(),
                workgroup: Some("KINOTEST".into())
            })
        );
        let joined = meta.replace("/Workgroup:KINOTEST", "/NotJoined");
        assert_eq!(parse_metadata(&joined).unwrap().workgroup, None);
        let friendly = meta.replace("<pub:Computer>KINO-WINPC/Workgroup:KINOTEST</pub:Computer>", "");
        assert_eq!(parse_metadata(&friendly).unwrap().name, "WSD Device KINO-WINPC");
        assert!(parse_metadata("not xml").is_none());
    }

    #[test]
    fn messages_are_well_formed() {
        for m in [
            probe_message(ID),
            resolve_message(ID, "urn:uuid:<a>"),
            get_message(ID, "urn:uuid:a"),
        ] {
            let root = xml::parse(&m).unwrap();
            assert_eq!(root.text_of("MessageID"), Some(ID));
        }
        assert!(resolve_message(ID, "urn:uuid:<a>").contains("urn:uuid:&lt;a&gt;"));
    }
}
