// SPDX-License-Identifier: MPL-2.0
//! How the catalog reaches the files of a source (LIB-018).
//!
//! The scan, the change watcher and playback read every source through a
//! [`Vfs`] that a [`Connector`] builds from the `sources` row:
//!
//! - [`LocalConnector`] (the default) opens `file://` folders only;
//! - [`SourcesConnector`] opens every kind the source manager knows
//!   (`pa-sources`, NET-011): local folders, SMB shares through libsmb2
//!   and WebDAV collections, with the password from the secret store
//!   (`pa-secrets`). Nothing is mounted; the app never sees the password.
//!
//! [`SourceOpener`] serves files of any source to the loopback HTTP
//! proxy (`pa-proxy`, NET-007), so players that only take a URL can play
//! network files. A file with an offline copy on the device (NET-019) is
//! served from the copy.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

use pa_vfs::{ByteSource, ErrorKind, LocalFs, Vfs, VfsError, VfsPath};

use crate::{Catalog, Error, Source};

/// Builds the [`Vfs`] of a source.
///
/// Errors are shown to the user (scan status, Sources screen), so their
/// text names the problem and never a secret. A local folder that is not
/// there must fail with a message starting with `folder not found:` (the
/// app classifies by it).
pub trait Connector: Send + Sync + fmt::Debug {
    /// The file system rooted at `source`'s root. May connect lazily: an
    /// unreachable server can also show up on the first call of the `Vfs`.
    fn connect(&self, source: &Source) -> Result<Arc<dyn Vfs>, Error>;

    /// The music playlists the server of `source` keeps (MUS-012), with
    /// songs as paths of its tree; `None` (the default) for sources that
    /// have none. The scan mirrors them into audio playlists.
    fn server_playlists(
        &self,
        _source: &Source,
    ) -> Result<Option<Vec<pa_sources::ServerPlaylist>>, Error> {
        Ok(None)
    }
}

/// A local folder source, or the error the scan reports for it.
fn local(source: &Source) -> Result<Arc<dyn Vfs>, Error> {
    let Some(root) = &source.local_root else {
        return Err(Error::Io(format!("not a local folder: {}", source.uri)));
    };
    if !root.is_dir() {
        // Offline drive, unmounted disk: the scan keeps the items.
        return Err(Error::Io(format!("folder not found: {}", root.display())));
    }
    Ok(Arc::new(LocalFs::new(root)?))
}

/// Local folders only: network sources fail with an `unsupported` error.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalConnector;

impl Connector for LocalConnector {
    fn connect(&self, source: &Source) -> Result<Arc<dyn Vfs>, Error> {
        if source.local_root.is_some() {
            return local(source);
        }
        Err(Error::Io(format!(
            "{}: unsupported: no connection for {} sources here",
            source.name, source.kind
        )))
    }
}

/// Every source kind through [`pa_sources::SourceManager::connect`].
#[derive(Debug, Clone)]
pub struct SourcesConnector {
    manager: Arc<pa_sources::SourceManager>,
}

impl SourcesConnector {
    /// Connects through `manager`, which must work on the same library
    /// database as the catalog (the source ids are shared).
    #[must_use]
    pub fn new(manager: Arc<pa_sources::SourceManager>) -> Self {
        Self { manager }
    }
}

impl Connector for SourcesConnector {
    fn connect(&self, source: &Source) -> Result<Arc<dyn Vfs>, Error> {
        if source.local_root.is_some() {
            return local(source);
        }
        self.manager
            .connect(source.id)
            .map(Arc::from)
            .map_err(|e| Error::Io(e.to_string()))
    }

    fn server_playlists(
        &self,
        source: &Source,
    ) -> Result<Option<Vec<pa_sources::ServerPlaylist>>, Error> {
        if source.local_root.is_some() {
            return Ok(None);
        }
        self.manager
            .server_playlists(source.id)
            .map_err(|e| Error::Io(e.to_string()))
    }
}

impl Catalog {
    /// Replaces how sources are reached (default: [`LocalConnector`]).
    pub fn set_connector(&self, connector: Arc<dyn Connector>) {
        *self
            .connector
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = connector;
    }

    /// The current connector.
    #[must_use]
    pub fn connector(&self) -> Arc<dyn Connector> {
        Arc::clone(
            &self
                .connector
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// The file system of source `source_id`, through the connector.
    pub fn connect(&self, source_id: i64) -> Result<Arc<dyn Vfs>, Error> {
        let source = crate::sources::get(&self.lock(), source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        self.connector().connect(&source)
    }
}

/// Opens a media file of a source for playing or probing: the file
/// itself, or for a `.strm` file (NET-018) the remote file of the link it
/// holds (checked under `pa_vfs_http::LinkPolicy::local_file`: the LAN
/// and the Internet, never this device).
pub fn open_media(vfs: &dyn Vfs, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
    let file = vfs.open(path)?;
    if path.file_name().is_some_and(pa_vfs_http::is_strm) {
        return Ok(Box::new(pa_vfs_http::open_strm(file.as_ref())?));
    }
    Ok(file)
}

/// Files recently opened by [`SourceOpener`], kept for the next ranges.
const OPEN_FILES: usize = 4;

/// Serves `<source id>/<path inside the source>` to the loopback proxy
/// (`pa-proxy`): a player seeking in a network file sends a new range
/// request every time, so connections and open files are reused.
///
/// A source that fails is connected again on the next request (the share
/// came back, the password changed).
pub struct SourceOpener {
    catalog: Arc<Catalog>,
    state: Mutex<OpenerState>,
}

#[derive(Default)]
struct OpenerState {
    vfs: HashMap<i64, Arc<dyn Vfs>>,
    files: VecDeque<(i64, VfsPath, Arc<dyn ByteSource>)>,
    /// Disc images opened as file systems (PLAY-017).
    images: VecDeque<(i64, VfsPath, Arc<pa_vfs::iso::IsoFs>)>,
}

/// Disc images kept open by [`SourceOpener`].
const OPEN_IMAGES: usize = 2;

impl fmt::Debug for SourceOpener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceOpener").finish_non_exhaustive()
    }
}

impl SourceOpener {
    /// An opener over the sources of `catalog`.
    #[must_use]
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            state: Mutex::new(OpenerState::default()),
        }
    }

    /// The proxy resource of a file: `<source id>/<path>` (not encoded).
    #[must_use]
    pub fn resource(source_id: i64, path: &VfsPath) -> String {
        format!("{source_id}{}", path.as_str())
    }

    pub(crate) fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub(crate) fn cached_image(&self, id: i64, image: &VfsPath) -> Option<Arc<pa_vfs::iso::IsoFs>> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.images
            .iter()
            .find(|(s, p, _)| *s == id && p == image)
            .map(|(_, _, fs)| Arc::clone(fs))
    }

    pub(crate) fn keep_image(&self, id: i64, image: &VfsPath, fs: Arc<pa_vfs::iso::IsoFs>) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.images.retain(|(s, p, _)| !(*s == id && p == image));
        st.images.push_front((id, image.clone(), fs));
        st.images.truncate(OPEN_IMAGES);
    }

    /// Opens `path` of source `id`, reusing a recent open of the same file.
    /// A file the active profile may not see (LIB-012) is not found. A
    /// path into a disc image (`Film.iso/BDMV/STREAM/00010.m2ts`) opens
    /// the file inside the image (PLAY-017).
    pub fn open_file(&self, id: i64, path: &VfsPath) -> pa_vfs::Result<Arc<dyn ByteSource>> {
        if let Some((image, inner)) = pa_vfs::iso::split_image_path(path) {
            return Ok(Arc::from(self.image(id, &image)?.open(&inner)?));
        }
        let allowed = self
            .catalog
            .may_open(id, path.as_str())
            .map_err(|e| VfsError::new(ErrorKind::Unavailable, e.to_string()))?;
        if !allowed {
            return Err(VfsError::new(ErrorKind::NotFound, "not in this profile"));
        }
        // NET-019: a downloaded copy plays without the network.
        if let Ok(Some(file)) = self.catalog.open_offline(id, path) {
            return Ok(Arc::from(file));
        }
        let cached_vfs = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, _, f)) = st.files.iter().find(|(s, p, _)| *s == id && p == path) {
                return Ok(Arc::clone(f));
            }
            st.vfs.get(&id).cloned()
        };
        let opened = match cached_vfs {
            Some(vfs) => match open_media(vfs.as_ref(), path) {
                Ok(f) => Ok(f),
                // A stale connection: connect once more below.
                Err(e) if e.kind() == ErrorKind::NotFound => Err(e),
                // A refused or broken link of a `.strm` file stays so.
                Err(e) if is_link_error(&e, path) => Err(e),
                Err(_) => open_media(self.fresh(id)?.as_ref(), path),
            },
            None => open_media(self.fresh(id)?.as_ref(), path),
        };
        let file: Arc<dyn ByteSource> = Arc::from(opened?);
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.files.push_front((id, path.clone(), Arc::clone(&file)));
        st.files.truncate(OPEN_FILES);
        Ok(file)
    }

    /// The connection to source `id` (cached, else a new one): for files
    /// next to a media file, such as trickplay previews (PLAY-009).
    pub fn vfs(&self, id: i64) -> pa_vfs::Result<Arc<dyn Vfs>> {
        let cached = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.vfs.get(&id).cloned()
        };
        match cached {
            Some(vfs) => Ok(vfs),
            None => self.fresh(id),
        }
    }

    fn fresh(&self, id: i64) -> pa_vfs::Result<Arc<dyn Vfs>> {
        let vfs = self.catalog.connect(id).map_err(|e| match e {
            Error::NotFound(m) => VfsError::new(ErrorKind::NotFound, m),
            e => VfsError::new(ErrorKind::Unavailable, e.to_string()),
        })?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.vfs.insert(id, Arc::clone(&vfs));
        st.files.retain(|(s, _, _)| *s != id);
        st.images.retain(|(s, _, _)| *s != id);
        Ok(vfs)
    }

    /// Forgets cached connections and open files (sources changed).
    pub fn reset(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.vfs.clear();
        st.files.clear();
        st.images.clear();
    }
}

/// A `.strm` link that the policy refused or that holds no link: a new
/// connection to the source would not change it.
fn is_link_error(e: &VfsError, path: &VfsPath) -> bool {
    path.file_name().is_some_and(pa_vfs_http::is_strm)
        && matches!(
            e.kind(),
            ErrorKind::PermissionDenied | ErrorKind::InvalidPath | ErrorKind::Unsupported
        )
}

impl pa_proxy::Opener for SourceOpener {
    fn open(&self, resource: &str) -> pa_vfs::Result<Arc<dyn ByteSource>> {
        let (id, path) = resource
            .split_once('/')
            .ok_or_else(|| VfsError::new(ErrorKind::InvalidPath, "no source id"))?;
        let id: i64 = id
            .parse()
            .map_err(|_| VfsError::new(ErrorKind::InvalidPath, "bad source id"))?;
        self.open_file(id, &VfsPath::parse(path)?)
    }
}
