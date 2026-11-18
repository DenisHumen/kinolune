// SPDX-License-Identifier: MPL-2.0
//! A media server's folders as a read-only `pa-vfs` tree.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, DirEntry, EntryKind, ErrorKind, Metadata, Vfs, VfsError, VfsPath};
use pa_vfs_http::{HttpClient, LinkPolicy, Timeouts, UniqueNames, Zone, file_name, sanitize};
use url::Url;

use crate::browse::{DidlObject, browse_all};
use crate::device::{MediaServer, fetch_server};

/// Titles of the "real folders" view several servers offer next to their
/// by-genre, by-artist… views (minidlna «Browse Folders», Serviio,
/// Windows Media Player, Universal Media Server «Folders»). When the root
/// has one, it is the root of the tree, so a file is not listed once per
/// view (ADR-0124).
pub const FOLDER_VIEWS: &[&str] = &[
    "browse folders",
    "folders",
    "media folders",
    "папки",
    "обзор папок",
];

/// How long a folder listing is reused.
const LISTING_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
enum Node {
    Dir { id: String },
    File { url: Url, size: u64 },
}

#[derive(Debug)]
struct Listing {
    at: Instant,
    entries: Vec<(String, Node)>,
}

/// A UPnP/DLNA media server read as folders of files (NET-018).
///
/// Folders are containers, files are the video and audio items with their
/// title and the media extension of their resource; images are left out.
/// Items without a size get one from a `HEAD`, and are left out when the
/// server does not give it. Files stream from the server's own `http-get`
/// URL with byte ranges, through the link policy of the server's zone
/// (a LAN server may only send the app to the LAN and the Internet).
pub struct DlnaFs {
    location: Url,
    client: Arc<HttpClient>,
    server: OnceLock<MediaServer>,
    root_id: OnceLock<String>,
    listings: Mutex<HashMap<String, Arc<Listing>>>,
}

impl fmt::Debug for DlnaFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DlnaFs")
            .field("location", &self.location.as_str())
            .finish_non_exhaustive()
    }
}

impl DlnaFs {
    /// The server whose description is at `location` (the source address).
    /// Nothing is requested until the first call.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidPath`] for a location that is not an `http(s)`
    /// link without credentials.
    pub fn new(location: Url) -> pa_vfs::Result<Self> {
        Self::with_timeouts(location, Timeouts::default())
    }

    /// Like [`new`](Self::new) with request timeouts.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_timeouts(location: Url, timeouts: Timeouts) -> pa_vfs::Result<Self> {
        pa_vfs_http::check_link(&location)?;
        // The server's own zone: its links may not reach anything less
        // public (a LAN server cannot send the app to this device).
        let zone = location
            .host_str()
            .and_then(|h| Zone::of_host(h, location.port_or_known_default().unwrap_or(80)))
            .unwrap_or(Zone::Lan);
        Ok(Self {
            client: Arc::new(HttpClient::with_timeouts(LinkPolicy::from(zone), timeouts)),
            location,
            server: OnceLock::new(),
            root_id: OnceLock::new(),
            listings: Mutex::new(HashMap::new()),
        })
    }

    /// The server (its description is read once).
    ///
    /// # Errors
    ///
    /// The server cannot be reached or is not a media server.
    pub fn server(&self) -> pa_vfs::Result<&MediaServer> {
        if let Some(s) = self.server.get() {
            return Ok(s);
        }
        let s = fetch_server(&self.client, &self.location)?;
        Ok(self.server.get_or_init(|| s))
    }

    /// The object id the tree starts at: a folder view of the root (see
    /// [`FOLDER_VIEWS`]) or the root `0` itself.
    fn root_id(&self) -> pa_vfs::Result<&str> {
        if let Some(id) = self.root_id.get() {
            return Ok(id);
        }
        let server = self.server()?;
        let children = browse_all(&self.client, &server.control_url, "0")?;
        let view = children.iter().find_map(|o| match o {
            DidlObject::Container { id, title }
                if FOLDER_VIEWS.contains(&title.trim().to_lowercase().as_str()) =>
            {
                Some(id.clone())
            }
            _ => None,
        });
        Ok(self.root_id.get_or_init(|| view.unwrap_or_else(|| "0".to_owned())))
    }

    /// The children of container `id`, named.
    fn listing(&self, id: &str) -> pa_vfs::Result<Arc<Listing>> {
        {
            let l = self.listings.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(listing) = l.get(id)
                && listing.at.elapsed() < LISTING_TTL
            {
                return Ok(Arc::clone(listing));
            }
        }
        let server = self.server()?;
        let objects = browse_all(&self.client, &server.control_url, id)?;
        let mut names = UniqueNames::default();
        let mut entries = Vec::with_capacity(objects.len());
        for o in objects {
            match o {
                DidlObject::Container { id, title } => {
                    entries.push((names.take(&sanitize(&title)), Node::Dir { id }));
                }
                DidlObject::Item {
                    title,
                    url,
                    size,
                    ext,
                    ..
                } => {
                    let size = match size.filter(|&s| s > 0) {
                        Some(s) => s,
                        None => match self.client.probe(&url).ok().and_then(|p| p.size) {
                            Some(s) if s > 0 => s,
                            _ => continue,
                        },
                    };
                    entries.push((names.take(&file_name(&title, ext)), Node::File { url, size }));
                }
            }
        }
        let listing = Arc::new(Listing {
            at: Instant::now(),
            entries,
        });
        self.listings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), Arc::clone(&listing));
        Ok(listing)
    }

    /// The node at `path`, walking down from the root.
    fn node(&self, path: &VfsPath) -> pa_vfs::Result<Node> {
        let mut node = Node::Dir {
            id: self.root_id()?.to_owned(),
        };
        for name in path.components() {
            let Node::Dir { id } = &node else {
                return Err(VfsError::new(
                    ErrorKind::NotFound,
                    format!("{path}: not found"),
                ));
            };
            let listing = self.listing(id)?;
            node = listing
                .entries
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, n)| n.clone())
                .ok_or_else(|| VfsError::new(ErrorKind::NotFound, format!("{path}: not found")))?;
        }
        Ok(node)
    }
}

fn metadata(node: &Node) -> Metadata {
    match node {
        Node::Dir { .. } => Metadata {
            kind: EntryKind::Dir,
            size: 0,
            modified_ms: None,
        },
        Node::File { size, .. } => Metadata {
            kind: EntryKind::File,
            size: *size,
            modified_ms: None,
        },
    }
}

impl Vfs for DlnaFs {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        let Node::Dir { id } = self.node(dir)? else {
            return Err(VfsError::new(
                ErrorKind::NotADirectory,
                format!("{dir}: a file"),
            ));
        };
        self.listing(&id)?
            .entries
            .iter()
            .map(|(name, node)| {
                Ok(DirEntry {
                    name: name.clone(),
                    path: dir.join(name)?,
                    metadata: metadata(node),
                })
            })
            .collect()
    }

    fn stat(&self, path: &VfsPath) -> pa_vfs::Result<Metadata> {
        self.node(path).map(|n| metadata(&n))
    }

    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        match self.node(path)? {
            Node::File { url, size } => Ok(Box::new(self.client.open(url, size)?)),
            Node::Dir { .. } => Err(VfsError::new(
                ErrorKind::IsADirectory,
                format!("{path}: a folder"),
            )),
        }
    }
}
