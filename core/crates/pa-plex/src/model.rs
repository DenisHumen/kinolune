// SPDX-License-Identifier: MPL-2.0
//! DTOs of plex.tv and of Plex Media Server, read with
//! `Accept: application/json`. Only the fields the client needs are read;
//! unknown fields are ignored, missing ones and `null` become defaults,
//! ids may come as strings or numbers, flags as booleans or `0`/`1`. Tokens
//! are kept out of `Debug`, e-mail addresses are not read at all.

use std::fmt;

use serde::{Deserialize, Deserializer};

/// A string, a number (as its text) or `null` (empty).
fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

/// `true`/`false`, `1`/`0`, `"1"`/`"0"`; anything else is `false`.
fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().is_some_and(|n| n != 0),
        serde_json::Value::String(s) => matches!(s.as_str(), "1" | "true"),
        _ => false,
    })
}

/// A number, or a number in a string; anything else is `None`.
fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

fn hidden(t: &Option<String>) -> &'static str {
    if t.is_some() { "Some(***)" } else { "None" }
}

/// A sign-in PIN of plex.tv (`/api/v2/pins`).
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pin {
    /// The PIN id to check.
    #[serde(deserialize_with = "lenient_i64")]
    pub id: Option<i64>,
    /// The code the user enters at plex.tv/link.
    pub code: String,
    /// Seconds until the code expires.
    #[serde(deserialize_with = "lenient_i64")]
    pub expires_in: Option<i64>,
    /// Set once the user linked the code: the account's token.
    pub auth_token: Option<String>,
}

impl fmt::Debug for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pin")
            .field("id", &self.id)
            .field("code", &self.code)
            .field("expires_in", &self.expires_in)
            .field("auth_token", &hidden(&self.auth_token))
            .finish()
    }
}

/// The signed-in plex.tv account (`/api/v2/user`). The e-mail address is
/// not read.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    /// Numeric account id.
    #[serde(deserialize_with = "lenient_i64")]
    pub id: Option<i64>,
    /// Account UUID.
    pub uuid: String,
    /// User name.
    pub username: String,
    /// Display name.
    pub title: String,
    /// Avatar URL.
    pub thumb: Option<String>,
    /// Whether the account is in a Plex Home.
    #[serde(deserialize_with = "lenient_bool")]
    pub home: bool,
    /// The account's token (the same as the PIN gave).
    pub auth_token: Option<String>,
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("uuid", &self.uuid)
            .field("title", &self.title)
            .field("home", &self.home)
            .field("auth_token", &hidden(&self.auth_token))
            .finish_non_exhaustive()
    }
}

impl Account {
    /// The name to show: the display name, else the user name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.username
        } else {
            &self.title
        }
    }
}

/// A user of a Plex Home (`/api/v2/home/users`). The e-mail address is
/// not read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HomeUser {
    /// Numeric user id.
    #[serde(deserialize_with = "lenient_i64")]
    pub id: Option<i64>,
    /// User UUID (what [`PlexTv::switch_user`](crate::PlexTv::switch_user)
    /// takes).
    pub uuid: String,
    /// Display name.
    pub title: String,
    /// User name (empty for managed users).
    pub username: String,
    /// Avatar URL.
    pub thumb: Option<String>,
    /// The Home's administrator (the owner of the account).
    #[serde(deserialize_with = "lenient_bool")]
    pub admin: bool,
    /// A guest user.
    #[serde(deserialize_with = "lenient_bool")]
    pub guest: bool,
    /// A managed user with restrictions (kids, teens).
    #[serde(deserialize_with = "lenient_bool")]
    pub restricted: bool,
    /// Switching to this user needs their PIN.
    #[serde(deserialize_with = "lenient_bool")]
    pub protected: bool,
}

impl HomeUser {
    /// The name to show: the display name, else the user name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.username
        } else {
            &self.title
        }
    }
}

/// `/api/v2/home/users` answers with `{ "users": [...] }`; an array is
/// accepted as well.
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum HomeUsers {
    Object {
        #[serde(default)]
        users: Vec<HomeUser>,
    },
    List(Vec<HomeUser>),
}

impl HomeUsers {
    pub(crate) fn into_vec(self) -> Vec<HomeUser> {
        match self {
            Self::Object { users } | Self::List(users) => users,
        }
    }
}

/// The answer of a user switch: the token of that user.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct SwitchedUser {
    pub(crate) auth_token: Option<String>,
    /// The name of the field in older answers.
    pub(crate) authentication_token: Option<String>,
}

/// One way to reach a server (`connections` of a resource).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Connection {
    /// `http` or `https`.
    pub protocol: String,
    /// IP address (or host).
    pub address: String,
    /// Port.
    #[serde(deserialize_with = "lenient_i64")]
    pub port: Option<i64>,
    /// Full URI (`https://1-2-3-4.<hash>.plex.direct:32400`).
    pub uri: String,
    /// On the LAN of the server.
    #[serde(deserialize_with = "lenient_bool")]
    pub local: bool,
    /// Through Plex's relay.
    #[serde(deserialize_with = "lenient_bool")]
    pub relay: bool,
    /// An IPv6 address.
    #[serde(rename = "IPv6", deserialize_with = "lenient_bool")]
    pub ipv6: bool,
}

/// A device of the account (`/api/v2/resources`); servers have
/// `provides` containing `server`.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Resource {
    /// Server name.
    pub name: String,
    /// `Plex Media Server`.
    pub product: String,
    /// Server version.
    pub product_version: String,
    /// OS of the server.
    pub platform: String,
    /// Machine identifier of the server.
    pub client_identifier: String,
    /// `server`, `client,player`, …
    pub provides: String,
    /// The user owns the server (else it is shared with them).
    #[serde(deserialize_with = "lenient_bool")]
    pub owned: bool,
    /// Online, as plex.tv last saw it.
    #[serde(deserialize_with = "lenient_bool")]
    pub presence: bool,
    /// The server refuses plain HTTP.
    #[serde(deserialize_with = "lenient_bool")]
    pub https_required: bool,
    /// The LAN's DNS hides `plex.direct` names (DNS rebinding
    /// protection): plain-IP connections work, the `plex.direct` ones not.
    #[serde(deserialize_with = "lenient_bool")]
    pub dns_rebinding_protection: bool,
    /// The token for this server (a shared server has its own).
    pub access_token: Option<String>,
    /// How to reach it.
    pub connections: Vec<Connection>,
}

impl fmt::Debug for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resource")
            .field("name", &self.name)
            .field("client_identifier", &self.client_identifier)
            .field("provides", &self.provides)
            .field("owned", &self.owned)
            .field("connections", &self.connections)
            .field("access_token", &hidden(&self.access_token))
            .finish_non_exhaustive()
    }
}

impl Resource {
    /// A Plex Media Server.
    #[must_use]
    pub fn is_server(&self) -> bool {
        self.provides.split(',').any(|p| p.trim() == "server")
    }
}

/// `/identity` of a server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Identity {
    /// Machine identifier (the resource's `clientIdentifier`).
    pub machine_identifier: String,
    /// Server version.
    pub version: String,
    /// Signed in to a plex.tv account.
    #[serde(deserialize_with = "lenient_bool")]
    pub claimed: bool,
}

/// A library of a server (`/library/sections`, a `Directory`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Section {
    /// Section id.
    #[serde(deserialize_with = "lenient_string")]
    pub key: String,
    /// Name.
    pub title: String,
    /// `movie`, `show`, `artist`, `photo`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Metadata agent (`tv.plex.agents.none` for "other videos").
    pub agent: String,
    /// Library UUID.
    pub uuid: String,
}

/// A tag; for `Guid` entries the external id (`tmdb://438631`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Tag {
    /// The value.
    #[serde(deserialize_with = "lenient_string")]
    pub id: String,
}

/// One file of a media version (`Part`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Part {
    /// Part id.
    #[serde(deserialize_with = "lenient_string")]
    pub id: String,
    /// Path of the file on the server's HTTP API
    /// (`/library/parts/1/1700000000/file.mkv`).
    pub key: String,
    /// Path of the file on the server's disk.
    pub file: Option<String>,
    /// Size in bytes.
    #[serde(deserialize_with = "lenient_i64")]
    pub size: Option<i64>,
    /// `mkv`, `mp4`, …
    pub container: Option<String>,
    /// Duration, ms.
    #[serde(deserialize_with = "lenient_i64")]
    pub duration: Option<i64>,
}

/// A version of an item (`Media`): one or more files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Media {
    /// Media id.
    #[serde(deserialize_with = "lenient_string")]
    pub id: String,
    /// Duration, ms.
    #[serde(deserialize_with = "lenient_i64")]
    pub duration: Option<i64>,
    /// Container of the version.
    pub container: Option<String>,
    /// `1080`, `4k`, `sd`, …
    #[serde(deserialize_with = "lenient_string")]
    pub video_resolution: String,
    /// Overall bit rate, kbit/s.
    #[serde(deserialize_with = "lenient_i64")]
    pub bitrate: Option<i64>,
    /// Edition or version title, if the server has one.
    pub title: Option<String>,
    /// The files (more than one: a movie split into parts).
    #[serde(rename = "Part")]
    pub parts: Vec<Part>,
}

/// A movie, show, episode, artist, album or track (`Metadata`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    /// Item id.
    #[serde(deserialize_with = "lenient_string")]
    pub rating_key: String,
    /// `movie`, `show`, `episode`, `track`, …
    #[serde(rename = "type")]
    pub kind: String,
    /// Title.
    pub title: String,
    /// Original title (a track: its own artist).
    pub original_title: Option<String>,
    /// Release year.
    #[serde(deserialize_with = "lenient_i64")]
    pub year: Option<i64>,
    /// Poster (`/library/metadata/1/thumb/1700000000`).
    pub thumb: Option<String>,
    /// Backdrop.
    pub art: Option<String>,
    /// Season or album poster.
    pub parent_thumb: Option<String>,
    /// Episode: season number; track: disc number.
    #[serde(deserialize_with = "lenient_i64")]
    pub parent_index: Option<i64>,
    /// Episode: episode number; track: track number.
    #[serde(deserialize_with = "lenient_i64")]
    pub index: Option<i64>,
    /// Episode: season; track: album.
    pub parent_title: Option<String>,
    /// Track: album id.
    #[serde(deserialize_with = "lenient_string")]
    pub parent_rating_key: String,
    /// Episode: show; track: album artist.
    pub grandparent_title: Option<String>,
    /// Episode: show id.
    #[serde(deserialize_with = "lenient_string")]
    pub grandparent_rating_key: String,
    /// When the server added it, Unix seconds.
    #[serde(deserialize_with = "lenient_i64")]
    pub added_at: Option<i64>,
    /// Duration, ms.
    #[serde(deserialize_with = "lenient_i64")]
    pub duration: Option<i64>,
    /// Resume position of the user, ms.
    #[serde(deserialize_with = "lenient_i64")]
    pub view_offset: Option<i64>,
    /// How often the user watched it.
    #[serde(deserialize_with = "lenient_i64")]
    pub view_count: Option<i64>,
    /// When the user last watched it, Unix seconds.
    #[serde(deserialize_with = "lenient_i64")]
    pub last_viewed_at: Option<i64>,
    /// External ids (with `includeGuids=1`).
    #[serde(rename = "Guid")]
    pub guids: Vec<Tag>,
    /// Versions and their files.
    #[serde(rename = "Media")]
    pub media: Vec<Media>,
}

impl Item {
    /// The external id of `provider` (`tmdb`, `imdb`, `tvdb`), if known.
    #[must_use]
    pub fn guid(&self, provider: &str) -> Option<&str> {
        self.guids.iter().find_map(|g| {
            g.id.split_once("://")
                .filter(|(p, v)| *p == provider && !v.is_empty())
                .map(|(_, v)| v)
        })
    }
}

/// A playlist (`/playlists`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlist {
    /// Playlist id.
    #[serde(deserialize_with = "lenient_string")]
    pub rating_key: String,
    /// Name.
    pub title: String,
    /// `audio`, `video`, `photo`.
    pub playlist_type: String,
    /// Built from rules.
    #[serde(deserialize_with = "lenient_bool")]
    pub smart: bool,
    /// Number of items.
    #[serde(deserialize_with = "lenient_i64")]
    pub leaf_count: Option<i64>,
}

/// `{ "MediaContainer": { ... } }` of the server.
#[derive(Deserialize)]
pub(crate) struct Envelope<T> {
    #[serde(rename = "MediaContainer")]
    pub(crate) container: Container<T>,
}

/// A page of a listing.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Container<T> {
    /// Items on the whole listing, when paged.
    #[serde(default, deserialize_with = "lenient_i64")]
    pub(crate) total_size: Option<i64>,
    #[serde(rename = "Metadata", default = "Vec::new")]
    pub(crate) metadata: Vec<T>,
    #[serde(rename = "Directory", default = "Vec::new")]
    pub(crate) directory: Vec<T>,
}

/// `/identity`.
#[derive(Deserialize)]
pub(crate) struct IdentityEnvelope {
    #[serde(rename = "MediaContainer", default)]
    pub(crate) container: Identity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lenient_values_and_hides_tokens() {
        let pin: Pin = serde_json::from_str(
            r#"{"id":"42","code":"AB12","expiresIn":900,"authToken":"secret","x":1}"#,
        )
        .unwrap();
        assert_eq!(pin.id, Some(42));
        assert!(!format!("{pin:?}").contains("secret"));
        let r: Resource = serde_json::from_str(
            r#"{"name":"NAS","provides":"server,player","owned":1,"accessToken":"t0k","connections":[{"uri":"http://a:1","local":"1","IPv6":true,"port":"32400"}]}"#,
        )
        .unwrap();
        assert!(r.is_server() && r.owned);
        assert!(r.connections[0].local && r.connections[0].ipv6);
        assert_eq!(r.connections[0].port, Some(32400));
        assert!(!format!("{r:?}").contains("t0k"));
        let a: Account =
            serde_json::from_str(r#"{"uuid":"u","username":"anna","title":"","email":"x@y"}"#)
                .unwrap();
        assert_eq!(a.display_name(), "anna");
        let item: Item = serde_json::from_str(
            r#"{"ratingKey":7,"type":"movie","title":"Dune","year":null,"Guid":[{"id":"tmdb://438631"},{"id":"imdb://tt1160419"},{"id":"plex://movie/x"}]}"#,
        )
        .unwrap();
        assert_eq!(item.rating_key, "7");
        assert_eq!(item.year, None);
        assert_eq!(item.guid("tmdb"), Some("438631"));
        assert_eq!(item.guid("tvdb"), None);
        let users: HomeUsers = serde_json::from_str(r#"[{"uuid":"a"}]"#).unwrap();
        assert_eq!(users.into_vec().len(), 1);
    }
}
