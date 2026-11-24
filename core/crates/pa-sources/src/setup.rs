// SPDX-License-Identifier: MPL-2.0
//! Setting up a new device from a phone (UI-046): the setup bundle and its
//! sources.
//!
//! The phone puts every network source a new device can use as it is —
//! SMB, WebDAV, SFTP, FTP, Subsonic, DLNA, playlists — with its password
//! into a [`SetupBundle`], next to the app's settings. `pa_sync::setup`
//! seals the bundle with the family key and hands it to the new device
//! right after pairing; there [`SourceManager::import_setup`] adds the
//! sources, their passwords go only into the new device's secret store.
//!
//! Left out: local folders (a path means nothing on another device),
//! personal media (UI-025, never leaves the device), Twitch archives, and
//! Jellyfin, Emby and Plex, whose secret is a sign-in token bound to the
//! phone as a client: the new device signs in itself (onboarding finds
//! these servers on the LAN).
//!
//! The bundle is never logged: `Debug` hides passwords, and its JSON lives
//! only between [`SetupBundle::to_json`] and the seal (the caller keeps it
//! in a `Zeroizing` buffer).

use std::collections::BTreeMap;
use std::fmt;

use pa_library::rusqlite::params;
use pa_secrets::SecretString;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    Error, NewSource, PasswordChange, Result, SourceKind, SourceManager, SourceOptions,
    SourceUpdate, address,
};

/// Version of the bundle format.
pub const BUNDLE_VERSION: u32 = 1;
/// Most sources in a bundle.
pub const MAX_SOURCES: usize = 256;
/// Most settings in a bundle.
pub const MAX_SETTINGS: usize = 256;
/// Longest value of a setting, in bytes.
pub const MAX_SETTING_LEN: usize = 16 * 1024;

/// Kinds a new device takes over as they are.
const KINDS: &[SourceKind] = &[
    SourceKind::Smb,
    SourceKind::WebDav,
    SourceKind::Sftp,
    SourceKind::Ftp,
    SourceKind::Subsonic,
    SourceKind::Dlna,
    SourceKind::Playlist,
];

fn ser_secret<S: Serializer>(v: &Option<SecretString>, s: S) -> std::result::Result<S::Ok, S::Error> {
    match v {
        Some(p) => s.serialize_some(p.expose()),
        None => s.serialize_none(),
    }
}

fn de_secret<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<SecretString>, D::Error> {
    Ok(Option::<String>::deserialize(d)?
        .filter(|p| !p.is_empty())
        .map(SecretString::new))
}

/// A source in the bundle. `Debug` does not print the password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupSource {
    /// [`SourceKind::as_str`].
    pub kind: String,
    pub name: String,
    /// Normalized URI without credentials.
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// The password (or SSH key secret, or API key), in clear text inside
    /// the sealed bundle only.
    #[serde(
        default,
        serialize_with = "ser_secret",
        deserialize_with = "de_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub password: Option<SecretString>,
    #[serde(default)]
    pub options: SourceOptions,
    pub scan_into_library: bool,
    pub auto_scan: bool,
    pub enabled: bool,
}

impl fmt::Debug for SetupSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupSource")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("uri", &self.uri)
            .field("has_password", &self.password.is_some())
            .finish_non_exhaustive()
    }
}

/// What a phone hands a new device: sources and the app's settings (the
/// app decides which; they are not secret, but travel sealed all the same).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupBundle {
    pub v: u32,
    #[serde(default)]
    pub sources: Vec<SetupSource>,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
}

impl SetupBundle {
    /// A bundle of `sources` and `settings`.
    pub fn new(sources: Vec<SetupSource>, settings: BTreeMap<String, String>) -> Self {
        Self {
            v: BUNDLE_VERSION,
            sources,
            settings,
        }
    }

    /// The JSON to seal. It holds passwords: keep it in a zeroizing
    /// buffer and never log it.
    pub fn to_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| Error::Invalid(format!("bundle: {e}")))
    }

    /// Reads an opened bundle and checks its limits.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let bundle: Self = serde_json::from_slice(bytes)
            // The serde error may quote the input: say only where it broke.
            .map_err(|e| Error::Invalid(format!("bundle at line {} column {}", e.line(), e.column())))?;
        if bundle.v != BUNDLE_VERSION {
            return Err(Error::Invalid(format!("bundle version {}", bundle.v)));
        }
        if bundle.sources.len() > MAX_SOURCES || bundle.settings.len() > MAX_SETTINGS {
            return Err(Error::Invalid("bundle too large".into()));
        }
        if bundle
            .settings
            .iter()
            .any(|(k, v)| k.len() > 128 || v.len() > MAX_SETTING_LEN)
        {
            return Err(Error::Invalid("bundle setting too long".into()));
        }
        Ok(bundle)
    }
}

/// What [`SourceManager::import_setup`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetupImport {
    /// New sources.
    pub added: u32,
    /// Sources this device had without a password that got one.
    pub updated: u32,
    /// Sources this device already had, and ones it cannot use.
    pub skipped: u32,
}

impl SourceManager {
    /// The sources a new device takes over (see the module), with their
    /// passwords from the secret store. A password that cannot be read
    /// leaves the source without one: the new device asks for it.
    pub fn export_setup(&self) -> Result<Vec<SetupSource>> {
        let ids: Vec<i64> = {
            let conn = self.lock();
            let kinds: Vec<&str> = KINDS.iter().map(|k| k.as_str()).collect();
            let mut stmt = conn.prepare(&format!(
                "SELECT id FROM sources
                  WHERE content_type <> 'personal' AND kind IN ({})
                  ORDER BY position, id",
                kinds
                    .iter()
                    .map(|k| format!("'{k}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            stmt.query_map(params![], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let stored = self.stored(id)?;
            let password = self.password_of(&stored).ok().flatten();
            let s = stored.source;
            out.push(SetupSource {
                kind: s.kind.as_str().to_owned(),
                name: s.name,
                uri: s.uri,
                username: s.username,
                password,
                options: s.options,
                scan_into_library: s.scan_into_library,
                auto_scan: s.auto_scan,
                enabled: s.enabled,
            });
        }
        Ok(out)
    }

    /// Adds the sources of a setup bundle. A source of the same kind and
    /// address that is already here is kept as it is; it only gets the
    /// password if it has none. One that cannot be added (an unknown kind
    /// from a newer phone, a bad address) is skipped. New sources stay on
    /// this device (sync off), as they were added here by hand.
    pub fn import_setup(&self, sources: &[SetupSource]) -> Result<SetupImport> {
        let mut report = SetupImport::default();
        let existing = self.list()?;
        for s in sources {
            let Ok(kind) = s.kind.parse::<SourceKind>() else {
                report.skipped += 1;
                continue;
            };
            if !KINDS.contains(&kind) {
                report.skipped += 1;
                continue;
            }
            let Ok(uri) = address::normalize(kind, &s.uri) else {
                report.skipped += 1;
                continue;
            };
            if let Some(have) = existing.iter().find(|e| e.kind == kind && e.uri == uri) {
                match &s.password {
                    Some(p) if !have.has_password && !kind.is_open() => {
                        self.update(
                            have.id,
                            SourceUpdate {
                                password: PasswordChange::Set(p.clone()),
                                ..SourceUpdate::default()
                            },
                        )?;
                        report.updated += 1;
                    }
                    _ => report.skipped += 1,
                }
                continue;
            }
            let new = NewSource {
                kind,
                name: s.name.clone(),
                address: uri,
                username: s.username.clone(),
                password: s.password.clone().filter(|_| !kind.is_open()),
                options: s.options.clone(),
                scan_into_library: s.scan_into_library,
                auto_scan: s.auto_scan,
                enabled: s.enabled,
                sync: crate::SourceSync::Off,
            };
            match self.create(new) {
                Ok(_) => report.added += 1,
                // A broken entry must not stop the others; the secret store
                // failing stops everything (the next ones would fail too).
                Err(e @ Error::Secrets(_)) => return Err(e),
                Err(_) => report.skipped += 1,
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_errors_never_show_the_password() {
        let s = SetupSource {
            kind: "smb".into(),
            name: "NAS".into(),
            uri: "smb://nas/media".into(),
            username: Some("me".into()),
            password: Some(SecretString::new("hunter2-UI046")),
            options: SourceOptions::default(),
            scan_into_library: true,
            auto_scan: true,
            enabled: true,
        };
        let bundle = SetupBundle::new(vec![s], BTreeMap::new());
        assert!(!format!("{bundle:?}").contains("hunter2"));
        let json = bundle.to_json().unwrap();
        assert_eq!(SetupBundle::from_json(&json).unwrap(), bundle);
        // A damaged bundle: the error does not quote it.
        let mut broken = json.clone();
        broken.truncate(json.len() - 3);
        let err = SetupBundle::from_json(&broken).unwrap_err().to_string();
        assert!(!err.contains("hunter2"), "{err}");
        let other = br#"{"v":2,"sources":[]}"#;
        assert!(SetupBundle::from_json(other).is_err());
    }
}
