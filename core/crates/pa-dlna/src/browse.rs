// SPDX-License-Identifier: MPL-2.0
//! ContentDirectory `Browse` and the DIDL-Lite it answers with.

use pa_vfs::{ErrorKind, VfsError};
use pa_vfs_http::{HttpClient, extension_of_mime, extension_of_url};
use url::Url;

use crate::xml::{self, El};

/// Objects asked for per `Browse` call.
pub const PAGE: u32 = 200;
/// Children of one container that are read at most.
pub const MAX_CHILDREN: usize = 20_000;
/// Largest `Browse` answer read.
const MAX_ANSWER_BYTES: u64 = 8 * 1024 * 1024;

const SOAP_ACTION: &str = "\"urn:schemas-upnp-org:service:ContentDirectory:1#Browse\"";

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// `object.item.videoItem…`
    Video,
    /// `object.item.audioItem…`
    Audio,
}

/// One child of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DidlObject {
    /// A folder.
    Container {
        /// Object id, to browse it.
        id: String,
        /// `dc:title`.
        title: String,
    },
    /// A video or audio file with a resource the app can stream.
    Item {
        /// Object id.
        id: String,
        /// `dc:title`.
        title: String,
        /// Video or audio.
        kind: ItemKind,
        /// The `http-get` resource, not a transcode.
        url: Url,
        /// `res@size`, if the server gives it.
        size: Option<u64>,
        /// The media extension of the resource (its URL, else its MIME
        /// type).
        ext: &'static str,
    },
}

/// The SOAP body of `Browse(object_id, BrowseDirectChildren)`.
#[must_use]
pub fn browse_request(object_id: &str, start: u32, count: u32) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
<s:Body><u:Browse xmlns:u=\"urn:schemas-upnp-org:service:ContentDirectory:1\">\
<ObjectID>{}</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>*</Filter>\
<StartingIndex>{start}</StartingIndex><RequestedCount>{count}</RequestedCount><SortCriteria></SortCriteria>\
</u:Browse></s:Body></s:Envelope>",
        xml::escape(object_id)
    )
}

/// One page of a `Browse` answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Page {
    /// The playable children of the page.
    pub objects: Vec<DidlObject>,
    /// `NumberReturned`: objects in the page, playable or not.
    pub returned: u32,
    /// `TotalMatches`; some servers say 0 when they do not know.
    pub total: u32,
}

/// Reads a `Browse` answer (status and body of the SOAP call).
///
/// # Errors
///
/// [`ErrorKind::NotFound`] for UPnP error 701 (no such object),
/// [`ErrorKind::Other`] for other faults and answers that are not a
/// `BrowseResponse`.
pub fn parse_browse(status: u16, text: &str) -> pa_vfs::Result<Page> {
    let env = xml::parse(text, "Browse answer")?;
    if let Some(fault) = env.find("Fault") {
        let code = fault
            .find("errorCode")
            .map(|c| c.text.trim().to_owned())
            .unwrap_or_default();
        let kind = match code.as_str() {
            "701" | "710" => ErrorKind::NotFound,
            "401" | "606" => ErrorKind::PermissionDenied,
            _ => ErrorKind::Other,
        };
        return Err(VfsError::new(kind, format!("UPnP error {code}")));
    }
    if status != 200 {
        return Err(VfsError::new(ErrorKind::Other, format!("HTTP {status}")));
    }
    let resp = env
        .find("BrowseResponse")
        .ok_or_else(|| VfsError::new(ErrorKind::Other, "not a Browse answer"))?;
    let num = |name: &str| resp.text_of(name).and_then(|v| v.parse().ok()).unwrap_or(0);
    let didl = resp.text_of("Result").unwrap_or("");
    Ok(Page {
        objects: if didl.is_empty() {
            Vec::new()
        } else {
            parse_didl(didl)?
        },
        returned: num("NumberReturned"),
        total: num("TotalMatches"),
    })
}

/// The containers and playable items of a DIDL-Lite document. Images,
/// playlists, items without an `http-get` resource and references to other
/// items (`refID`, the same file in another view) are left out.
///
/// # Errors
///
/// [`ErrorKind::Other`] for XML that is not DIDL-Lite.
pub fn parse_didl(didl: &str) -> pa_vfs::Result<Vec<DidlObject>> {
    let root = xml::parse(didl, "DIDL-Lite")?;
    let mut out = Vec::new();
    for el in &root.children {
        let id = el.attr("id").unwrap_or("").to_owned();
        let title = el.text_of("title").unwrap_or("").to_owned();
        match el.name.as_str() {
            "container" if !id.is_empty() => out.push(DidlObject::Container { id, title }),
            "item" if el.attr("refID").is_none_or(str::is_empty) => {
                if let Some(item) = item(el, id, title) {
                    out.push(item);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn item(el: &El, id: String, title: String) -> Option<DidlObject> {
    let class = el.text_of("class").unwrap_or("");
    let kind = if class.starts_with("object.item.videoItem") {
        ItemKind::Video
    } else if class.starts_with("object.item.audioItem") {
        ItemKind::Audio
    } else {
        return None;
    };
    // The original file: an http-get resource that is not a conversion
    // (DLNA.ORG_CI=1 marks a transcode).
    let mut best: Option<(&El, Url, &str)> = None;
    for res in el.all("res") {
        let info = res.attr("protocolInfo").unwrap_or("");
        let mut parts = info.splitn(4, ':');
        let (proto, _, mime, extra) = (
            parts.next().unwrap_or(""),
            parts.next(),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        if !proto.eq_ignore_ascii_case("http-get") {
            continue;
        }
        let Ok(url) = Url::parse(res.text.trim()) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let transcoded = extra.contains("DLNA.ORG_CI=1");
        if best.is_none() || !transcoded && best.as_ref().is_some_and(|(r, ..)| is_transcode(r)) {
            best = Some((res, url, mime));
        }
    }
    let (res, url, mime) = best?;
    let ext = extension_of_url(&url)
        .or_else(|| extension_of_mime(mime))
        .unwrap_or(match kind {
            ItemKind::Video => "mkv",
            ItemKind::Audio => "mp3",
        });
    Some(DidlObject::Item {
        id,
        title,
        kind,
        size: res.attr("size").and_then(|s| s.trim().parse().ok()),
        url,
        ext,
    })
}

fn is_transcode(res: &El) -> bool {
    res.attr("protocolInfo")
        .is_some_and(|i| i.contains("DLNA.ORG_CI=1"))
}

/// Every child of container `object_id`, page by page (at most
/// [`MAX_CHILDREN`]).
///
/// # Errors
///
/// The server cannot be reached, or the answer is a fault.
pub fn browse_all(
    client: &HttpClient,
    control: &Url,
    object_id: &str,
) -> pa_vfs::Result<Vec<DidlObject>> {
    let mut out = Vec::new();
    let mut start = 0u32;
    loop {
        let (status, text) = client.post_xml(
            control,
            SOAP_ACTION,
            &browse_request(object_id, start, PAGE),
            MAX_ANSWER_BYTES,
        )?;
        let page = parse_browse(status, &text)?;
        out.extend(page.objects);
        start += page.returned;
        let more = if page.total > 0 {
            start < page.total
        } else {
            page.returned == PAGE
        };
        if page.returned == 0 || !more || out.len() >= MAX_CHILDREN {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIDL: &str = r#"<DIDL-Lite xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/" xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/">
<container id="64$0" parentID="64" restricted="1"><dc:title>Movies</dc:title><upnp:class>object.container.storageFolder</upnp:class></container>
<item id="64$1$0" parentID="64$1" restricted="1"><dc:title>Song</dc:title><upnp:class>object.item.audioItem.musicTrack</upnp:class><res size="24483" protocolInfo="http-get:*:audio/mpeg:DLNA.ORG_PN=MP3">http://172.17.0.11:8200/MediaItems/25.mp3</res></item>
<item id="v1" parentID="1"><dc:title>Film</dc:title><upnp:class>object.item.videoItem.movie</upnp:class><res protocolInfo="http-get:*:video/mp4:DLNA.ORG_CI=1">http://h/transcode/1</res><res size="99" protocolInfo="http-get:*:video/x-matroska:DLNA.ORG_CI=0">http://h/MediaItems/1</res></item>
<item id="r1" parentID="2" refID="v1"><dc:title>Film</dc:title><upnp:class>object.item.videoItem</upnp:class><res protocolInfo="http-get:*:video/mp4:*">http://h/1.mp4</res></item>
<item id="p1" parentID="3"><dc:title>Photo</dc:title><upnp:class>object.item.imageItem.photo</upnp:class><res protocolInfo="http-get:*:image/jpeg:*">http://h/1.jpg</res></item>
<item id="x1" parentID="3"><dc:title>RTP</dc:title><upnp:class>object.item.videoItem</upnp:class><res protocolInfo="rtsp-rtp-udp:*:video/mp4:*">rtsp://h/1</res></item>
</DIDL-Lite>"#;

    #[test]
    fn reads_didl() {
        let objs = parse_didl(DIDL).unwrap();
        assert_eq!(objs.len(), 3, "{objs:?}");
        assert_eq!(
            objs[0],
            DidlObject::Container {
                id: "64$0".into(),
                title: "Movies".into()
            }
        );
        match &objs[1] {
            DidlObject::Item {
                title,
                kind,
                size,
                ext,
                url,
                ..
            } => {
                assert_eq!(title, "Song");
                assert_eq!(*kind, ItemKind::Audio);
                assert_eq!(*size, Some(24483));
                assert_eq!(*ext, "mp3");
                assert_eq!(url.as_str(), "http://172.17.0.11:8200/MediaItems/25.mp3");
            }
            o => panic!("{o:?}"),
        }
        match &objs[2] {
            DidlObject::Item { url, ext, size, .. } => {
                assert_eq!(
                    url.as_str(),
                    "http://h/MediaItems/1",
                    "the original, not the transcode"
                );
                assert_eq!(*ext, "mkv");
                assert_eq!(*size, Some(99));
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn reads_browse_answers_and_faults() {
        let body = format!(
            "<s:Envelope xmlns:s=\"x\"><s:Body><u:BrowseResponse xmlns:u=\"y\"><Result>{}</Result><NumberReturned>6</NumberReturned><TotalMatches>0</TotalMatches></u:BrowseResponse></s:Body></s:Envelope>",
            xml::escape(DIDL)
        );
        let page = parse_browse(200, &body).unwrap();
        assert_eq!(page.objects.len(), 3);
        assert_eq!((page.returned, page.total), (6, 0));

        let fault = "<s:Envelope xmlns:s=\"x\"><s:Body><s:Fault><detail><UPnPError><errorCode>701</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>";
        assert_eq!(
            parse_browse(500, fault).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        assert!(browse_request("a&b", 0, 10).contains("<ObjectID>a&amp;b</ObjectID>"));
    }
}
