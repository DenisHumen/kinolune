// SPDX-License-Identifier: MPL-2.0
//! Media servers that answer a UDP question of their own:
//!
//! - **Plex GDM** ("G'Day Mate"): `M-SEARCH * HTTP/1.1` to
//!   `239.0.0.250:32414` (and the broadcast address); the answer is HTTP
//!   headers with `Content-Type: plex/media-server`, `Name`, `Port`,
//!   `Resource-Identifier`.
//! - **Jellyfin / Emby auto-discovery**: `who is JellyfinServer?` or
//!   `who is EmbyServer?` broadcast to UDP 7359; the answer is JSON with
//!   `Address`, `Id`, `Name`.
//!
//! The address of the source is always built on the IP that answered:
//! a server cannot send the app to another host.

use std::net::{Ipv4Addr, SocketAddrV4};

use url::Url;

/// The Plex GDM multicast group and server port.
pub const GDM_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 0, 0, 250), 32414);

/// The Jellyfin and Emby auto-discovery port.
pub const AUTODISCOVERY_PORT: u16 = 7359;

/// The Plex GDM search.
pub const GDM_REQUEST: &[u8] = b"M-SEARCH * HTTP/1.1\r\n\r\n";

/// The Jellyfin question.
pub const JELLYFIN_REQUEST: &[u8] = b"who is JellyfinServer?";

/// The Emby question.
pub const EMBY_REQUEST: &[u8] = b"who is EmbyServer?";

/// A media server that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAnswer {
    /// Its name.
    pub name: String,
    /// Its id (Plex `Resource-Identifier`, Jellyfin/Emby `Id`), if given.
    pub id: Option<String>,
    /// Where it answers HTTP: on the IP that answered.
    pub url: Url,
}

fn on_host(scheme: &str, host: Ipv4Addr, port: u16) -> Option<Url> {
    Url::parse(&format!("{scheme}://{host}:{port}")).ok()
}

/// Reads a Plex GDM answer from `from`.
#[must_use]
pub fn parse_gdm(datagram: &[u8], from: Ipv4Addr) -> Option<ServerAnswer> {
    let text = std::str::from_utf8(datagram).ok()?;
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let status = lines.next()?;
    if !status.starts_with("HTTP/1.") || !status.contains(" 200") {
        return None;
    }
    let (mut kind, mut name, mut port, mut id) = (None, None, 32400u16, None);
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "content-type" => kind = Some(v.to_ascii_lowercase()),
            "name" => name = Some(v.to_owned()),
            "port" => port = v.parse().ok().filter(|p| *p != 0)?,
            "resource-identifier" => id = Some(v.to_owned()),
            _ => {}
        }
    }
    // Plex players answer GDM too (plex/media-player): not sources.
    if kind.as_deref() != Some("plex/media-server") {
        return None;
    }
    Some(ServerAnswer {
        name: name.unwrap_or_default(),
        id,
        url: on_host("http", from, port)?,
    })
}

/// Reads a Jellyfin or Emby auto-discovery answer from `from`. The
/// scheme and port come from its `Address`; the host is always `from`
/// (behind a reverse proxy the `Address` may name another host, and the
/// app must not go there on a LAN device's word).
#[must_use]
pub fn parse_autodiscovery(datagram: &[u8], from: Ipv4Addr) -> Option<ServerAnswer> {
    let v: serde_json::Value = serde_json::from_slice(datagram).ok()?;
    let obj = v.as_object()?;
    let text = |k: &str| {
        obj.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let address = text("Address").and_then(|a| Url::parse(a).ok());
    let (scheme, port) = match &address {
        Some(a) if matches!(a.scheme(), "http" | "https") => (
            a.scheme().to_owned(),
            a.port_or_known_default().unwrap_or(8096),
        ),
        _ => ("http".to_owned(), 8096),
    };
    // Something must say it is a server: an Id or an Address.
    if text("Id").is_none() && address.is_none() {
        return None;
    }
    Some(ServerAnswer {
        name: text("Name").unwrap_or_default().to_owned(),
        id: text("Id").map(str::to_owned),
        url: on_host(&scheme, from, port)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 20);

    #[test]
    fn reads_plex_servers_only() {
        let ok = b"HTTP/1.0 200 OK\r\nContent-Type: plex/media-server\r\nResource-Identifier: 0123abc\r\nName: Living Room\r\nPort: 32400\r\nUpdated-At: 1700000000\r\nVersion: 1.40.0\r\n\r\n";
        let a = parse_gdm(ok, FROM).unwrap();
        assert_eq!(a.name, "Living Room");
        assert_eq!(a.id.as_deref(), Some("0123abc"));
        assert_eq!(a.url.as_str(), "http://192.168.1.20:32400/");
        let player = b"HTTP/1.0 200 OK\r\nContent-Type: plex/media-player\r\nName: TV\r\nPort: 32500\r\n\r\n";
        assert!(parse_gdm(player, FROM).is_none());
        let bad_port = b"HTTP/1.0 200 OK\r\nContent-Type: plex/media-server\r\nPort: 99999\r\n\r\n";
        assert!(parse_gdm(bad_port, FROM).is_none());
        assert!(parse_gdm(b"M-SEARCH * HTTP/1.1\r\n\r\n", FROM).is_none());
        assert!(parse_gdm(b"\xff", FROM).is_none());
    }

    #[test]
    fn reads_jellyfin_answers_on_the_sender() {
        // As Jellyfin 10.10.7 answers (captured in the NET-010 LAN test).
        let ok = br#"{"Address":"http://172.17.0.12:8096","Id":"77e85e6587c14cc0aabf74eddbe1544a","Name":"50637ccc3f9a","EndpointAddress":null}"#;
        let a = parse_autodiscovery(ok, Ipv4Addr::new(172, 17, 0, 12)).unwrap();
        assert_eq!(a.url.as_str(), "http://172.17.0.12:8096/");
        assert_eq!(a.name, "50637ccc3f9a");
        assert_eq!(a.id.as_deref(), Some("77e85e6587c14cc0aabf74eddbe1544a"));
        // Another host in Address: the scheme and port are kept, the host
        // is the sender.
        let proxied = br#"{"Address":"https://media.example.com:8920","Id":"x","Name":"Home"}"#;
        let a = parse_autodiscovery(proxied, FROM).unwrap();
        assert_eq!(a.url.as_str(), "https://192.168.1.20:8920/");
        let no_port = br#"{"Address":"https://media.example.com","Id":"x"}"#;
        assert_eq!(
            parse_autodiscovery(no_port, FROM).unwrap().url.as_str(),
            "https://192.168.1.20/"
        );
        let odd = br#"{"Address":"ftp://x","Id":"x"}"#;
        assert_eq!(
            parse_autodiscovery(odd, FROM).unwrap().url.as_str(),
            "http://192.168.1.20:8096/"
        );
        assert!(parse_autodiscovery(br#"{"Name":"x"}"#, FROM).is_none());
        assert!(parse_autodiscovery(b"[1,2]", FROM).is_none());
        assert!(parse_autodiscovery(b"who is JellyfinServer?", FROM).is_none());
    }
}
