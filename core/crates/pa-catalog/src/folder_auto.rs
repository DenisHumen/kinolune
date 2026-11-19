// SPDX-License-Identifier: MPL-2.0
//! The sync folder picks itself (SYNC-015, ADR-0129 "Автоматический выбор
//! папки").
//!
//! Without a choice of the user every round of folder sync
//! ([`Catalog::run_folder_sync`]) first checks the folder:
//!
//! 1. A source that already holds journals of other devices in
//!    `/.kinolune/state/` is joined. When several do, the one with more
//!    devices that wrote in the last [`ACTIVE_MS`] wins, then more devices
//!    at all; ties go to the folder holding the smallest device id, then
//!    the smaller source address. Every device sees the same journals, so
//!    every device ranks the folders the same way.
//! 2. Else the first source in the order they were added that can be
//!    written gets `/.kinolune/state/` and this device's journal.
//!
//! Only sources other devices can reach count ([`can_carry`]): SMB,
//! WebDAV, SFTP, FTP, and local folders that are network mounts. Media
//! servers, DLNA, playlists and plain local folders do not.
//!
//! The pick is looked at again only when there is no folder yet, when
//! the sources changed, when the folder has no other devices yet (every
//! [`REEVAL_MS`], to join the others once they show up) and when it has
//! been unreachable for [`SWITCH_AFTER_MS`]. A folder that has other
//! devices and can be reached is kept: no flapping. Moving on, the own
//! ops of the folder left behind (if it can be read) go to the new one.

use std::cmp::Reverse;
use std::path::Path;
use std::sync::Arc;

use pa_library::rusqlite::{OptionalExtension, params};
use pa_sync::folder::{Folder, JournalInfo};
use pa_vfs::Vfs;

use crate::{Catalog, Error, now_ms};

/// Kinds of network sources that can carry the folder.
const NETWORK_KINDS: [&str; 4] = ["smb", "webdav", "sftp", "ftp"];
/// A journal written in this window counts as an active device.
pub const ACTIVE_MS: i64 = 14 * 24 * 60 * 60 * 1000;
/// How often a folder without other devices looks for one that has them.
pub const REEVAL_MS: i64 = 5 * 60 * 1000;
/// How long an automatic folder may be unreachable before the next one
/// takes over.
pub const SWITCH_AFTER_MS: i64 = 10 * 60 * 1000;

/// A source that could carry the folder, in the order the sources were
/// added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) id: i64,
    pub(crate) uri: String,
}

/// What a look into one candidate found.
struct Survey {
    candidate: Candidate,
    order: usize,
    vfs: Arc<dyn Vfs>,
    journals: Vec<JournalInfo>,
}

/// File systems of network mounts (`/proc/self/mountinfo` types).
const NETWORK_FS: [&str; 15] = [
    "cifs",
    "smb3",
    "smbfs",
    "nfs",
    "nfs4",
    "afs",
    "ceph",
    "glusterfs",
    "davfs",
    "ncpfs",
    "fuse.sshfs",
    "fuse.rclone",
    "fuse.davfs2",
    "fuse.s3fs",
    "fuse.glusterfs",
];

/// Undoes the octal escapes of `/proc/self/mountinfo` (`\040` = space).
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let code = b[i + 1..i + 4]
                .iter()
                .fold(0u32, |acc, c| acc * 8 + u32::from(c - b'0'));
            out.push(u8::try_from(code).unwrap_or(b'?'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `path` lies on a network mount according to `mountinfo` (the
/// text of `/proc/self/mountinfo`): the deepest mount point above it has a
/// network file system, or is a GVfs share of a network server.
pub(crate) fn on_network_mount(mountinfo: &str, path: &Path) -> bool {
    let mut best: Option<(usize, String, String)> = None;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let Some(point) = left.split(' ').nth(4) else {
            continue;
        };
        let Some(fstype) = right.split(' ').next() else {
            continue;
        };
        let point = unescape_mount(point);
        if !path.starts_with(&point) {
            continue;
        }
        if best.as_ref().is_none_or(|(len, _, _)| point.len() >= *len) {
            best = Some((point.len(), point, fstype.to_owned()));
        }
    }
    let Some((_, point, fstype)) = best else {
        return false;
    };
    if NETWORK_FS.contains(&fstype.as_str()) {
        return true;
    }
    // GNOME mounts shares under `…/gvfs/smb-share:server=…,share=…`.
    fstype == "fuse.gvfsd-fuse"
        && path
            .strip_prefix(&point)
            .ok()
            .and_then(|rest| rest.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .is_some_and(|share| {
                ["smb-share:", "dav:", "davs:", "sftp:", "ftp:", "nfs:"]
                    .iter()
                    .any(|p| share.starts_with(p))
            })
}

/// Whether the local folder `path` is a network mount (Linux and Android;
/// elsewhere local folders never count).
fn is_network_mount(path: &Path) -> bool {
    if cfg!(any(target_os = "linux", target_os = "android")) {
        std::fs::read_to_string("/proc/self/mountinfo")
            .is_ok_and(|text| on_network_mount(&text, path))
    } else {
        false
    }
}

/// Whether a source of `kind` (with `local_root` for local folders) can
/// carry the folder for the other devices.
pub(crate) fn can_carry(kind: &str, local_root: Option<&Path>) -> bool {
    NETWORK_KINDS.contains(&kind) || (kind == "local" && local_root.is_some_and(is_network_mount))
}

type Rank = (
    usize,
    usize,
    Reverse<String>,
    Reverse<String>,
    Reverse<usize>,
);

/// Ranks the folders that hold journals: more active devices, then more
/// devices, then the smallest device id, then the smaller address, then
/// the earlier source. Larger is better.
fn rank(s: &Survey, now: i64) -> Rank {
    let active = s
        .journals
        .iter()
        .filter(|j| j.modified_ms.is_none_or(|t| now.saturating_sub(t) < ACTIVE_MS))
        .count();
    let smallest = s
        .journals
        .iter()
        .map(|j| j.device.clone())
        .min()
        .unwrap_or_default();
    (
        active,
        s.journals.len(),
        Reverse(smallest),
        Reverse(s.candidate.uri.clone()),
        Reverse(s.order),
    )
}

/// The folder to join: the best of those holding journals of other
/// devices, with the current one (`current`) in the running too; `None`
/// when no folder has other devices.
fn pick_join(me: &str, current: Option<i64>, surveys: &[Survey], now: i64) -> Option<i64> {
    let others = |s: &Survey| s.journals.iter().any(|j| j.device != me);
    if !surveys.iter().any(others) {
        return None;
    }
    surveys
        .iter()
        .filter(|s| others(s) || Some(s.candidate.id) == current)
        .max_by_key(|s| rank(s, now))
        .map(|s| s.candidate.id)
}

impl Catalog {
    /// Sources that could carry the folder, earliest added first.
    pub(crate) fn auto_sources(&self) -> Result<Vec<Candidate>, Error> {
        let rows: Vec<(i64, String, String)> = {
            let conn = self.lock();
            let mut stmt = conn.prepare(
                "SELECT id, kind, uri FROM sources
                  WHERE enabled AND kind <> 'twitch_archive'
                  ORDER BY created_at_ms, id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        Ok(rows
            .into_iter()
            .filter(|(_, kind, uri)| {
                can_carry(kind, crate::uri::file_uri_to_path(uri).as_deref())
            })
            .map(|(id, _, uri)| Candidate { id, uri })
            .collect())
    }

    /// Checks the automatic folder before a round (see the module docs).
    /// Does nothing when the user picked a folder or turned it off.
    /// Callers hold [`Catalog::folder_lock`].
    pub(crate) fn auto_pick(&self, me: &str) -> Result<(), Error> {
        let row = self.folder_row()?;
        if row.as_ref().is_some_and(|r| !r.auto) || (row.is_none() && self.folder_off()?) {
            return Ok(());
        }
        let now = now_ms();
        let candidates = self.auto_sources()?;
        let seen = candidates
            .iter()
            .map(|c| c.id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let (evaluated, last_seen): (Option<i64>, String) = self
            .lock()
            .query_row(
                "SELECT evaluated_ms, sources FROM sync_folder_choice WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((None, String::new()));

        let current = row
            .as_ref()
            .filter(|r| candidates.iter().any(|c| c.id == r.source_id));
        let away = current.is_some_and(|r| {
            r.failing_since_ms
                .is_some_and(|t| now.saturating_sub(t) >= SWITCH_AFTER_MS)
        });
        let due = evaluated.is_none_or(|t| now.saturating_sub(t) >= REEVAL_MS) || last_seen != seen;
        let look = match current {
            None => true,
            Some(_) if away => true,
            Some(r) => r.devices == 0 && due,
        };
        if !look {
            return Ok(());
        }
        let current_id = current.map(|r| r.source_id);

        let mut surveys = Vec::new();
        for (order, candidate) in candidates.into_iter().enumerate() {
            if away && Some(candidate.id) == current_id {
                continue;
            }
            let Ok(vfs) = self.connect(candidate.id) else {
                continue;
            };
            let Ok(journals) = Folder::new(vfs.as_ref()).journals() else {
                continue;
            };
            surveys.push(Survey {
                candidate,
                order,
                vfs,
                journals,
            });
        }
        self.lock().execute(
            "INSERT INTO sync_folder_choice (id, mode, evaluated_ms, sources)
             VALUES (1, 'auto', ?1, ?2)
             ON CONFLICT (id) DO UPDATE SET
                 evaluated_ms = excluded.evaluated_ms, sources = excluded.sources",
            params![now, seen],
        )?;

        let kept = current_id.filter(|_| !away);
        let target = match pick_join(me, kept, &surveys, now) {
            Some(id) => Some(id),
            None if kept.is_some() => kept,
            // Make the folder on the first source that takes a journal.
            None => {
                let now_u = u64::try_from(now).unwrap_or(0);
                surveys.iter().find_map(|s| {
                    match Folder::new(s.vfs.as_ref()).write(me, &[], now_u) {
                        Ok(w) if !w.read_only => Some(s.candidate.id),
                        _ => None,
                    }
                })
            }
        };
        match target {
            Some(id) if Some(id) != row.as_ref().map(|r| r.source_id) => {
                // The own ops of the folder left behind go along, if it
                // can be read.
                let carried = row
                    .as_ref()
                    .and_then(|r| surveys.iter().find(|s| s.candidate.id == r.source_id))
                    .and_then(|s| Folder::new(s.vfs.as_ref()).own_ops(me).ok())
                    .unwrap_or_default();
                tracing::info!(
                    target: "pa_catalog::folder_sync",
                    source = id,
                    carried = carried.len(),
                    "sync folder picked automatically"
                );
                self.select_folder(id, me, true)?;
                self.queue_for_folder(&carried)?;
            }
            Some(_) => {}
            // No folder anywhere; a folder that may no longer carry the
            // sync (a source turned off or not a network mount) is let go.
            None if row.is_some() && current.is_none() => {
                let conn = self.lock();
                conn.execute("DELETE FROM sync_folder", [])?;
                conn.execute("DELETE FROM sync_folder_outbox", [])?;
            }
            None => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
40 22 0:40 / /mnt/nas rw,relatime shared:20 - cifs //nas/video rw,vers=3.1.1
41 22 0:41 / /mnt/nas/local\\040disk rw shared:21 - ext4 /dev/sdb1 rw
42 22 0:42 / /media/nfs rw shared:22 - nfs4 nas:/export rw
43 22 0:43 / /run/user/1000/gvfs rw shared:23 - fuse.gvfsd-fuse gvfsd-fuse rw
44 22 0:44 / /home/u/remote rw shared:24 - fuse.sshfs u@nas:/srv rw
";

    #[test]
    fn network_mounts_are_told_from_local_disks() {
        let net = |p: &str| on_network_mount(MOUNTINFO, Path::new(p));
        assert!(net("/mnt/nas"));
        assert!(net("/mnt/nas/Movies"));
        assert!(!net("/mnt/nas/local disk/Movies"));
        assert!(!net("/mnt/nasty"));
        assert!(net("/media/nfs/x"));
        assert!(net("/home/u/remote/media"));
        assert!(!net("/home/u/Videos"));
        assert!(net(
            "/run/user/1000/gvfs/smb-share:server=nas,share=video/Movies"
        ));
        assert!(!net("/run/user/1000/gvfs/mtp:host=phone/DCIM"));
        assert!(!on_network_mount("", Path::new("/mnt/nas")));
    }

    #[test]
    fn only_sources_other_devices_reach_can_carry_the_folder() {
        for kind in ["smb", "webdav", "sftp", "ftp"] {
            assert!(can_carry(kind, None), "{kind}");
        }
        for kind in [
            "dlna", "playlist", "plex", "jellyfin", "emby", "subsonic", "local",
        ] {
            assert!(!can_carry(kind, None), "{kind}");
        }
    }

    fn survey(id: i64, order: usize, journals: &[(&str, Option<i64>)]) -> Survey {
        Survey {
            candidate: Candidate {
                id,
                uri: format!("smb://nas{id}/share"),
            },
            order,
            vfs: Arc::new(pa_vfs::LocalFs::new(std::env::temp_dir()).unwrap()),
            journals: journals
                .iter()
                .map(|(d, t)| JournalInfo {
                    device: (*d).to_owned(),
                    size: 10,
                    modified_ms: *t,
                })
                .collect(),
        }
    }

    #[test]
    fn the_folder_with_more_active_devices_wins_and_ties_break_the_same_everywhere() {
        let now = 100 * ACTIVE_MS;
        let fresh = Some(now - 1000);
        let stale = Some(now - 2 * ACTIVE_MS);
        // No other devices anywhere: nothing to join.
        let alone = [survey(1, 0, &[("me", fresh)]), survey(2, 1, &[])];
        assert_eq!(pick_join("me", Some(1), &alone, now), None);
        // Two active devices beat three stale ones.
        let s = [
            survey(1, 0, &[("a", stale), ("b", stale), ("c", stale)]),
            survey(2, 1, &[("a", fresh), ("b", fresh)]),
        ];
        assert_eq!(pick_join("me", None, &s, now), Some(2));
        // Stale only: more devices win over nothing.
        let s = [survey(1, 0, &[]), survey(2, 1, &[("a", stale)])];
        assert_eq!(pick_join("me", Some(1), &s, now), Some(2));
        // Two devices that each made a folder: both pick the same one.
        let s = [survey(1, 0, &[("ka", fresh)]), survey(2, 1, &[("kb", fresh)])];
        assert_eq!(pick_join("ka", Some(1), &s, now), Some(1));
        assert_eq!(pick_join("kb", Some(2), &s, now), Some(1));
        // The current folder holds with the same rank.
        let s = [survey(1, 0, &[("x", fresh)]), survey(2, 1, &[("x", fresh)])];
        assert_eq!(pick_join("me", None, &s, now), Some(1));
    }
}
