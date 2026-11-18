// SPDX-License-Identifier: MPL-2.0
//! UPnP device descriptions: the name of a media server and where its
//! ContentDirectory answers.

use pa_vfs::{ErrorKind, VfsError};
use pa_vfs_http::HttpClient;
use url::Url;

use crate::xml;

/// Largest device description read.
const MAX_DESCRIPTION_BYTES: u64 = 512 * 1024;

/// A UPnP media server with a ContentDirectory service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaServer {
    /// `friendlyName` ("Living-room NAS").
    pub name: String,
    /// `UDN` (`uuid:…`), stable across restarts and address changes.
    pub udn: String,
    /// Where the description was read: the address of the source.
    pub location: Url,
    /// The ContentDirectory `controlURL`, absolute.
    pub control_url: Url,
    /// `manufacturer`, if given.
    pub manufacturer: Option<String>,
    /// `modelName`, if given.
    pub model: Option<String>,
}

impl MediaServer {
    /// The host of the server, for the UI.
    #[must_use]
    pub fn host(&self) -> &str {
        self.location.host_str().unwrap_or("")
    }
}

/// Reads a device description (`location` is where it came from, the
/// base of relative URLs when it has no `URLBase`). Embedded devices are
/// searched too: some NAS put the media server inside a root device.
///
/// # Errors
///
/// [`ErrorKind::Unsupported`] if no device has a ContentDirectory (a
/// renderer, a router); [`ErrorKind::Other`] for XML that is not a
/// description.
pub fn parse_description(text: &str, location: &Url) -> pa_vfs::Result<MediaServer> {
    let root = xml::parse(text, "device description")?;
    let base = root
        .text_of("URLBase")
        .and_then(|b| Url::parse(b).ok())
        .unwrap_or_else(|| location.clone());
    let mut stack: Vec<&xml::El> = root.all("device").collect();
    while let Some(device) = stack.pop() {
        if let Some(services) = device.child("serviceList") {
            for s in services.all("service") {
                let ty = s.text_of("serviceType").unwrap_or("");
                if !ty.starts_with("urn:schemas-upnp-org:service:ContentDirectory:") {
                    continue;
                }
                let Some(control) = s.text_of("controlURL").and_then(|c| base.join(c).ok()) else {
                    continue;
                };
                let name = device
                    .text_of("friendlyName")
                    .map(str::to_owned)
                    .unwrap_or_else(|| location.host_str().unwrap_or("UPnP").to_owned());
                return Ok(MediaServer {
                    name,
                    udn: device.text_of("UDN").unwrap_or("").to_owned(),
                    location: location.clone(),
                    control_url: control,
                    manufacturer: device.text_of("manufacturer").map(str::to_owned),
                    model: device.text_of("modelName").map(str::to_owned),
                });
            }
        }
        if let Some(list) = device.child("deviceList") {
            stack.extend(list.all("device"));
        }
    }
    Err(VfsError::new(
        ErrorKind::Unsupported,
        "the device is not a media server (no ContentDirectory)",
    ))
}

/// Reads the description at `location` with `client`.
///
/// # Errors
///
/// The server cannot be reached, or [`parse_description`] fails.
pub fn fetch_server(client: &HttpClient, location: &Url) -> pa_vfs::Result<MediaServer> {
    let (text, _) = client.get_text(location, MAX_DESCRIPTION_BYTES)?;
    parse_description(&text, location)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_minidlna_and_embedded_devices() {
        let loc = Url::parse("http://192.168.1.5:8200/rootDesc.xml").unwrap();
        let s = parse_description(
            r#"<?xml version="1.0"?><root xmlns="urn:schemas-upnp-org:device-1-0"><device><deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType><friendlyName>NAS &amp; Co</friendlyName><manufacturer>Justin Maggard</manufacturer><modelName>MiniDLNA</modelName><UDN>uuid:1</UDN><serviceList><service><serviceType>urn:schemas-upnp-org:service:ConnectionManager:1</serviceType><controlURL>/ctl/ConnectionMgr</controlURL></service><service><serviceType>urn:schemas-upnp-org:service:ContentDirectory:1</serviceType><controlURL>/ctl/ContentDir</controlURL></service></serviceList></device></root>"#,
            &loc,
        )
        .unwrap();
        assert_eq!(s.name, "NAS & Co");
        assert_eq!(s.udn, "uuid:1");
        assert_eq!(
            s.control_url.as_str(),
            "http://192.168.1.5:8200/ctl/ContentDir"
        );
        assert_eq!(s.model.as_deref(), Some("MiniDLNA"));
        assert_eq!(s.host(), "192.168.1.5");

        let s = parse_description(
            r#"<root><URLBase>http://10.0.0.2:9000/dev/</URLBase><device><friendlyName>Root</friendlyName><deviceList><device><friendlyName>Media</friendlyName><UDN>uuid:2</UDN><serviceList><service><serviceType>urn:schemas-upnp-org:service:ContentDirectory:2</serviceType><controlURL>cd/control</controlURL></service></serviceList></device></deviceList></device></root>"#,
            &loc,
        )
        .unwrap();
        assert_eq!(s.name, "Media");
        assert_eq!(
            s.control_url.as_str(),
            "http://10.0.0.2:9000/dev/cd/control"
        );

        let e = parse_description(
            "<root><device><friendlyName>TV</friendlyName><serviceList/></device></root>",
            &loc,
        )
        .unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Unsupported);
    }
}
