// SPDX-License-Identifier: MPL-2.0
//! MUS-012: playlists of a music server mirrored by the scan. A local
//! folder stands in for the server's tree; a connector gives its
//! "server playlists" as paths, as `pa-sources` does for Navidrome and
//! Jellyfin (live: `pa-subsonic/tests/navidrome/run.sh`).

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::{Catalog, Connector, Error, LocalConnector, ProfileSettings, Source};
use pa_scanner::CancelToken;
use pa_sources::ServerPlaylist;
use pa_vfs::Vfs;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

/// Local folders with the playlists a test sets.
#[derive(Debug, Default)]
struct Server {
    lists: Mutex<Option<Vec<ServerPlaylist>>>,
}

impl Connector for Server {
    fn connect(&self, source: &Source) -> Result<Arc<dyn Vfs>, Error> {
        LocalConnector.connect(source)
    }

    fn server_playlists(&self, _source: &Source) -> Result<Option<Vec<ServerPlaylist>>, Error> {
        Ok(self.lists.lock().unwrap().clone())
    }
}

fn list(id: &str, name: &str, paths: &[&str]) -> ServerPlaylist {
    ServerPlaylist {
        remote_id: id.to_owned(),
        name: name.to_owned(),
        paths: paths.iter().map(|p| (*p).to_owned()).collect(),
        video: false,
    }
}

const ONE: &str = "/Artist/Album/01 - One.flac";
const TWO: &str = "/Artist/Album/02 - Two.flac";
const THREE: &str = "/Band/Single/01 - Three.flac";

fn titles(cat: &Catalog, id: i64) -> Vec<String> {
    cat.music_playlist_items(id)
        .unwrap()
        .into_iter()
        .map(|i| i.rel_path)
        .collect()
}

#[test]
fn scan_mirrors_server_playlists_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, ONE.trim_start_matches('/'), 1);
    media(&root, TWO.trim_start_matches('/'), 2);
    media(&root, THREE.trim_start_matches('/'), 3);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let server = Arc::new(Server::default());
    cat.set_connector(server.clone());
    let src = cat
        .add_source(None, "local", "Navidrome", root.to_str().unwrap(), None)
        .unwrap();
    // A second profile that sees every source, and a kid profile limited
    // to no folder of it.
    let guest = cat
        .create_profile(
            &ProfileSettings {
                name: "Guest".into(),
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
    cat.create_profile(
        &ProfileSettings {
            name: "Kid".into(),
            is_kid: true,
            restrict_folders: true,
            ..ProfileSettings::default()
        },
        None,
    )
    .unwrap();
    let own = cat.create_music_playlist("Mine").unwrap();

    *server.lists.lock().unwrap() = Some(vec![
        // A song the server has in another library: left out.
        list("p1", "Road", &[THREE, "/Elsewhere/x.flac", ONE, THREE]),
        list("p2", "Empty", &[]),
    ]);
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();

    let lists = cat.music_playlists().unwrap();
    assert_eq!(lists.len(), 3, "{lists:?}");
    let road = lists.iter().find(|p| p.name == "Road").unwrap();
    assert_eq!(road.server.as_deref(), Some("Navidrome"));
    assert_eq!(road.item_count, 2);
    let mine = lists.iter().find(|p| p.id == own.id).unwrap();
    assert_eq!(mine.server, None);
    // The server's order, each song once.
    let paths = titles(&cat, road.id);
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths[0].ends_with("Three.flac") && paths[1].ends_with("One.flac"));

    // Read-only: the next scan would undo any edit.
    let track = cat.music_tracks().unwrap()[0].item.id;
    assert!(matches!(
        cat.add_to_music_playlist(road.id, track),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        cat.rename_music_playlist(road.id, "x"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        cat.delete_music_playlist(road.id),
        Err(Error::Invalid(_))
    ));
    assert!(cat.add_to_music_playlist(own.id, track).unwrap());

    // The other profile has its own copy; the kid profile none.
    cat.switch_profile(guest.id, None, None).unwrap();
    let mut guest_lists: Vec<String> = cat
        .music_playlists()
        .unwrap()
        .into_iter()
        .map(|p| {
            assert_eq!(p.server.as_deref(), Some("Navidrome"));
            assert_ne!(p.id, road.id);
            p.name
        })
        .collect();
    guest_lists.sort();
    assert_eq!(guest_lists, ["Empty", "Road"]);
    let kid = cat
        .profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Kid")
        .unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();
    assert!(cat.music_playlists().unwrap().is_empty());
    cat.switch_profile(1, None, None).unwrap();

    // Renamed, reordered and one deleted on the server.
    *server.lists.lock().unwrap() = Some(vec![list("p1", "Road trip", &[TWO, ONE])]);
    cat.scan(Some(src.id), &CancelToken::new(), &mut |_| {})
        .unwrap();
    let lists = cat.music_playlists().unwrap();
    assert_eq!(lists.len(), 2, "{lists:?}");
    let road2 = lists.iter().find(|p| p.server.is_some()).unwrap();
    assert_eq!((road2.id, road2.name.as_str()), (road.id, "Road trip"));
    let paths = titles(&cat, road2.id);
    assert!(paths[0].ends_with("Two.flac") && paths[1].ends_with("One.flac"));

    // A server that cannot be reached keeps them (no playlists answer is
    // not "no playlists"): here the connector has none to give.
    *server.lists.lock().unwrap() = None;
    cat.scan(Some(src.id), &CancelToken::new(), &mut |_| {})
        .unwrap();
    assert_eq!(cat.music_playlists().unwrap().len(), 2);

    // Removing the source removes its playlists, not the user's.
    cat.remove_source(src.id).unwrap();
    let lists = cat.music_playlists().unwrap();
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert_eq!(lists[0].id, own.id);
}
