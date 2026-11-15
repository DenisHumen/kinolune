// SPDX-License-Identifier: MPL-2.0
//! Endpoints: server info, sign-in, users, libraries, items, Continue
//! Watching, favorites, played state, media segments and trickplay.
//!
//! Paths are those of Jellyfin 10.9+ (`/UserViews`, `/UserItems/Resume`,
//! `/UserFavoriteItems/{id}`, `/UserItems/{id}/UserData`, POST
//! `/QuickConnect/Initiate`); the integration tests pin 10.10.7. On Emby
//! ([`ServerKind::Emby`]) the user-scoped ones are the older
//! `/Users/{userId}/…` forms (`Views`, `Items`, `Items/Resume`,
//! `FavoriteItems/{id}`, `PlayedItems/{id}`), Continue Watching has a
//! real hide flag (`HideFromResume`) and there is no Quick Connect; the
//! integration tests pin Emby Server 4.10.2.

use serde_json::json;

use crate::http::{Client, Method, ServerKind, Token, encode};
use crate::model::{
    AuthenticationResult, Item, ItemPage, PublicSystemInfo, QuickConnectState, RawSegments,
    Segment, TrickplayInfo, User, segments_from_chapters, segments_json, ticks_to_ms,
};
use crate::{Error, Result};

/// Fields every item listing asks for: what the virtual tree, the
/// playback plan and the user data import need.
pub const ITEM_FIELDS: &str =
    "MediaSources,ProviderIds,Path,DateCreated,Trickplay,ParentId,Overview";

/// [`ITEM_FIELDS`] of Emby: chapters (with the intro and credits markers)
/// in place of trickplay, and the production year Emby leaves out by
/// default.
pub const EMBY_ITEM_FIELDS: &str =
    "MediaSources,ProviderIds,Path,DateCreated,Chapters,ParentId,Overview,ProductionYear";

/// Items fetched per request of a listing.
const PAGE: i64 = 500;

/// A signed-in user: who, on which server, with which token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The user.
    pub user: User,
    /// Server id.
    pub server_id: String,
    /// Access token; keep it in the secret store.
    pub token: Token,
}

impl From<AuthenticationResult> for Session {
    fn from(a: AuthenticationResult) -> Self {
        Self {
            server_id: a.server_id,
            user: a.user,
            token: Token::new(a.access_token),
        }
    }
}

/// A query for [`Client::items`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemQuery {
    /// Library or folder to look in; `None`: all libraries of the user.
    pub parent_id: Option<String>,
    /// Item types (`Movie`, `Series`, `Episode`, `Audio`, …); empty: any.
    pub types: Vec<String>,
    /// Look in subfolders too.
    pub recursive: bool,
    /// Only the user's favorites.
    pub favorites_only: bool,
    /// Text search.
    pub search: Option<String>,
}

impl Client {
    /// The item fields of this kind of server.
    fn fields(&self) -> &'static str {
        match self.kind() {
            ServerKind::Jellyfin => ITEM_FIELDS,
            ServerKind::Emby => EMBY_ITEM_FIELDS,
        }
    }

    fn emby(&self) -> bool {
        self.kind() == ServerKind::Emby
    }

    fn unsupported(&self, what: &str) -> Error {
        Error::Unsupported(format!("{what} on {}", self.kind().name()))
    }

    /// Name, id and version of the server; works without signing in.
    ///
    /// # Errors
    ///
    /// [`Error::Unreachable`] if no address answers.
    pub fn public_info(&self) -> Result<PublicSystemInfo> {
        self.get_json("/System/Info/Public")
    }

    /// Signs in with a user name and password and keeps the token.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] for a wrong name or password.
    pub fn sign_in(&self, username: &str, password: &str) -> Result<Session> {
        let a: AuthenticationResult = self.post_json(
            "/Users/AuthenticateByName",
            &json!({ "Username": username, "Pw": password }),
        )?;
        self.signed_in(a)
    }

    fn signed_in(&self, a: AuthenticationResult) -> Result<Session> {
        if a.access_token.is_empty() || a.user.id.is_empty() {
            return Err(Error::Protocol("sign-in answer without token".into()));
        }
        let s = Session::from(a);
        self.set_token(Some(s.token.clone()));
        self.set_user_id(Some(s.user.id.clone()));
        Ok(s)
    }

    /// Whether Quick Connect is switched on at the server.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn quick_connect_enabled(&self) -> Result<bool> {
        if self.emby() {
            // Emby's code sign-in goes through Emby Connect (Emby's cloud).
            return Ok(false);
        }
        self.get_json("/QuickConnect/Enabled")
    }

    /// Starts Quick Connect: show [`QuickConnectState::code`] to the user,
    /// who approves it in a signed-in client; then poll
    /// [`quick_connect_state`](Self::quick_connect_state) and finish with
    /// [`sign_in_quick_connect`](Self::sign_in_quick_connect).
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] if Quick Connect is switched off;
    /// [`Error::Unsupported`] on Emby.
    pub fn quick_connect_start(&self) -> Result<QuickConnectState> {
        if self.emby() {
            return Err(self.unsupported("Quick Connect"));
        }
        match self.post_json("/QuickConnect/Initiate", &json!({})) {
            // 10.8 and older take GET.
            Err(Error::NotFound | Error::Status { status: 405, .. }) => {
                self.get_json("/QuickConnect/Initiate")
            }
            other => other,
        }
    }

    /// Current state of a Quick Connect request.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] once the request expired.
    pub fn quick_connect_state(&self, secret: &str) -> Result<QuickConnectState> {
        if self.emby() {
            return Err(self.unsupported("Quick Connect"));
        }
        self.get_json(&format!("/QuickConnect/Connect?secret={}", encode(secret)))
    }

    /// Trades an approved Quick Connect secret for a token and keeps it.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] if the code has not been approved.
    pub fn sign_in_quick_connect(&self, secret: &str) -> Result<Session> {
        if self.emby() {
            return Err(self.unsupported("Quick Connect"));
        }
        let a: AuthenticationResult = self.post_json(
            "/Users/AuthenticateWithQuickConnect",
            &json!({ "Secret": secret }),
        )?;
        self.signed_in(a)
    }

    /// Approves a Quick Connect `code` shown by another device, as the
    /// signed-in user (sign a TV in from the phone).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown code.
    pub fn quick_connect_approve(&self, code: &str) -> Result<()> {
        if self.emby() {
            return Err(self.unsupported("Quick Connect"));
        }
        self.call(
            Method::Post,
            &format!("/QuickConnect/Authorize?code={}", encode(code)),
            None,
        )?;
        Ok(())
    }

    /// Ends the session of the token at the server and forgets it.
    ///
    /// # Errors
    ///
    /// Transport errors; the token is forgotten anyway.
    pub fn sign_out(&self) -> Result<()> {
        let r = self.call(Method::Post, "/Sessions/Logout", None);
        self.set_token(None);
        self.set_user_id(None);
        r.map(|_| ())
    }

    /// The signed-in user. Emby has no `/Users/Me`: there the user of the
    /// sign-in (or [`set_user_id`](Self::set_user_id)) is asked for.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] if the token is no longer valid;
    /// [`Error::Invalid`] on Emby without a known user.
    pub fn me(&self) -> Result<User> {
        if !self.emby() {
            return self.get_json("/Users/Me");
        }
        let id = self
            .user_id()
            .ok_or_else(|| Error::Invalid("no signed-in user".into()))?;
        self.user(&id)
    }

    /// User `user_id`.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown id.
    pub fn user(&self, user_id: &str) -> Result<User> {
        self.get_json(&format!("/Users/{}", encode(user_id)))
    }

    /// Users shown on the server's sign-in screen; no sign-in needed.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn public_users(&self) -> Result<Vec<User>> {
        self.get_json("/Users/Public")
    }

    /// All users (administrators only).
    ///
    /// # Errors
    ///
    /// [`Error::Forbidden`] for a user who is not an administrator.
    pub fn users(&self) -> Result<Vec<User>> {
        self.get_json("/Users")
    }

    /// Libraries of `user_id` (`CollectionFolder`s and the like).
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn libraries(&self, user_id: &str) -> Result<Vec<Item>> {
        let path = if self.emby() {
            format!("/Users/{}/Views", encode(user_id))
        } else {
            format!("/UserViews?userId={}", encode(user_id))
        };
        let page: ItemPage = self.get_json(&path)?;
        Ok(page.items)
    }

    /// One item as `user_id` sees it, with [`ITEM_FIELDS`].
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown id.
    pub fn item(&self, user_id: &str, item_id: &str) -> Result<Item> {
        let fields = self.fields();
        if self.emby() {
            return self.get_json(&format!(
                "/Users/{}/Items/{}?fields={fields}",
                encode(user_id),
                encode(item_id)
            ));
        }
        self.get_json(&format!(
            "/Items/{}?userId={}&fields={fields}",
            encode(item_id),
            encode(user_id)
        ))
    }

    /// All items matching `q`, fetched page by page, sorted by name.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn items(&self, user_id: &str, q: &ItemQuery) -> Result<Vec<Item>> {
        let scope = if self.emby() {
            format!("/Users/{}/Items?", encode(user_id))
        } else {
            format!("/Items?userId={}&", encode(user_id))
        };
        let mut base = format!(
            "{scope}fields={}&enableUserData=true\
             &enableImageTypes=Primary,Backdrop&imageTypeLimit=1&sortBy=SortName",
            self.fields()
        );
        if let Some(p) = &q.parent_id {
            base.push_str(&format!("&parentId={}", encode(p)));
        }
        if !q.types.is_empty() {
            base.push_str(&format!("&includeItemTypes={}", q.types.join(",")));
        }
        if q.recursive {
            base.push_str("&recursive=true");
        }
        if q.favorites_only {
            base.push_str("&isFavorite=true");
        }
        if let Some(s) = &q.search {
            base.push_str(&format!("&searchTerm={}", encode(s)));
        }
        let mut out = Vec::new();
        loop {
            let page: ItemPage =
                self.get_json(&format!("{base}&startIndex={}&limit={PAGE}", out.len()))?;
            let got = page.items.len();
            out.extend(page.items);
            if got == 0 || i64::try_from(out.len()).unwrap_or(i64::MAX) >= page.total_record_count {
                return Ok(out);
            }
        }
    }

    /// "Continue Watching" of `user_id`: videos with a resume position,
    /// latest first.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn resume(&self, user_id: &str, limit: u32) -> Result<Vec<Item>> {
        let scope = if self.emby() {
            format!("/Users/{}/Items/Resume?", encode(user_id))
        } else {
            format!("/UserItems/Resume?userId={}&", encode(user_id))
        };
        let page: ItemPage = self.get_json(&format!(
            "{scope}mediaTypes=Video&fields={}&enableUserData=true&limit={limit}",
            self.fields()
        ))?;
        Ok(page.items)
    }

    /// Removes `item_id` from Continue Watching. Jellyfin: drops the
    /// resume position and leaves "watched" and the play count as they are
    /// (what Jellyfin's own clients do; the server has no hide flag). Emby:
    /// sets its `HideFromResume` flag, the position stays.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn remove_from_resume(&self, user_id: &str, item_id: &str) -> Result<()> {
        if self.emby() {
            self.call(
                Method::Post,
                &format!(
                    "/Users/{}/Items/{}/HideFromResume?Hide=true",
                    encode(user_id),
                    encode(item_id)
                ),
                None,
            )?;
            return Ok(());
        }
        self.call(
            Method::Post,
            &format!(
                "/UserItems/{}/UserData?userId={}",
                encode(item_id),
                encode(user_id)
            ),
            Some(&json!({ "PlaybackPositionTicks": 0 })),
        )?;
        Ok(())
    }

    /// Marks `item_id` a favorite of `user_id`, or not.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn set_favorite(&self, user_id: &str, item_id: &str, favorite: bool) -> Result<()> {
        let path = if self.emby() {
            format!(
                "/Users/{}/FavoriteItems/{}",
                encode(user_id),
                encode(item_id)
            )
        } else {
            format!(
                "/UserFavoriteItems/{}?userId={}",
                encode(item_id),
                encode(user_id)
            )
        };
        let method = if favorite {
            Method::Post
        } else {
            Method::Delete
        };
        self.call(method, &path, None)?;
        Ok(())
    }

    /// Favorites of `user_id` (any type).
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn favorites(&self, user_id: &str) -> Result<Vec<Item>> {
        self.items(
            user_id,
            &ItemQuery {
                recursive: true,
                favorites_only: true,
                ..ItemQuery::default()
            },
        )
    }

    /// Marks `item_id` watched or not watched.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn set_played(&self, user_id: &str, item_id: &str, played: bool) -> Result<()> {
        let path = if self.emby() {
            format!("/Users/{}/PlayedItems/{}", encode(user_id), encode(item_id))
        } else {
            format!(
                "/UserPlayedItems/{}?userId={}",
                encode(item_id),
                encode(user_id)
            )
        };
        let method = if played { Method::Post } else { Method::Delete };
        self.call(method, &path, None)?;
        Ok(())
    }

    /// Media segments of `item_id` (intro, credits, …; Jellyfin 10.10+
    /// with a segment provider; empty on older servers). On Emby: the
    /// intro and credits markers of the item's chapters, as `user_id`
    /// sees the item.
    ///
    /// # Errors
    ///
    /// Transport errors.
    pub fn media_segments_of(&self, user_id: &str, item_id: &str) -> Result<Vec<Segment>> {
        if self.emby() {
            let item = self.item(user_id, item_id)?;
            return Ok(segments_from_chapters(
                &item.chapters,
                item.run_time_ticks.map(ticks_to_ms),
            ));
        }
        self.media_segments(item_id)
    }

    /// Jellyfin's media segments of `item_id` (see
    /// [`media_segments_of`](Self::media_segments_of), which works on Emby
    /// too).
    ///
    /// # Errors
    ///
    /// Transport errors; [`Error::Unsupported`] on Emby.
    pub fn media_segments(&self, item_id: &str) -> Result<Vec<Segment>> {
        if self.emby() {
            return Err(self.unsupported("media segments"));
        }
        match self.media_segments_json(item_id) {
            Ok(body) => {
                let raw: RawSegments = crate::http::parse(&body)?;
                Ok(raw.items.iter().map(Segment::from).collect())
            }
            Err(Error::NotFound) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// The answer of `/MediaSegments/{item_id}` as the server sent it (the
    /// format `pa_skip::import_markers` reads).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] on servers without the endpoint;
    /// [`Error::Unsupported`] on Emby (see
    /// [`media_segments_of`](Self::media_segments_of) and
    /// [`segments_json`](crate::segments_json)).
    pub fn media_segments_json(&self, item_id: &str) -> Result<Vec<u8>> {
        if self.emby() {
            return Err(self.unsupported("media segments"));
        }
        Ok(self
            .call(
                Method::Get,
                &format!("/MediaSegments/{}", encode(item_id)),
                None,
            )?
            .body)
    }

    /// Trickplay of a media source: the width closest to `want_width`, if
    /// the server made any.
    #[must_use]
    pub fn trickplay_of(
        item: &Item,
        media_source_id: &str,
        want_width: u32,
    ) -> Option<TrickplayInfo> {
        item.trickplay
            .get(media_source_id)
            .or_else(|| item.trickplay.values().next())?
            .values()
            .min_by_key(|t| t.width.abs_diff(want_width))
            .copied()
    }

    /// One trickplay sheet (a JPEG of `tile_width × tile_height` tiles).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] past the last sheet.
    pub fn trickplay_sheet(
        &self,
        item_id: &str,
        media_source_id: &str,
        width: u32,
        index: u32,
    ) -> Result<Vec<u8>> {
        self.get_bytes(&trickplay_path(item_id, media_source_id, width, index))
    }

    /// The preview thumbnails of `item_id` as a BIF file (Emby, when the
    /// server made them: "video preview thumbnails"), tiles `width` px
    /// wide. Emby answers an empty BIF for a video without thumbnails:
    /// that is [`Error::NotFound`] here.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] without thumbnails; [`Error::Unsupported`] on
    /// Jellyfin (it has trickplay sheets).
    pub fn bif(&self, item_id: &str, width: u32) -> Result<Vec<u8>> {
        if !self.emby() {
            return Err(self.unsupported("BIF thumbnails"));
        }
        let data = self.get_bytes(&bif_path(item_id, width))?;
        if bif_frames(&data) == 0 {
            return Err(Error::NotFound);
        }
        Ok(data)
    }

    /// An image of an item (`Primary`, `Backdrop`, …), at most
    /// `max_width` px wide, as JPEG.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if the item has no such image.
    pub fn image(
        &self,
        item_id: &str,
        image_type: &str,
        tag: Option<&str>,
        max_width: u32,
    ) -> Result<Vec<u8>> {
        self.get_bytes(&image_path(item_id, image_type, tag, max_width))
    }
}

pub(crate) fn trickplay_path(
    item_id: &str,
    media_source_id: &str,
    width: u32,
    index: u32,
) -> String {
    format!(
        "/Videos/{}/Trickplay/{width}/{index}.jpg?mediaSourceId={}",
        encode(item_id),
        encode(media_source_id)
    )
}

pub(crate) fn bif_path(item_id: &str, width: u32) -> String {
    format!("/Videos/{}/index.bif?Width={width}", encode(item_id))
}

/// Frames in a BIF (its header count), 0 for anything else.
pub(crate) fn bif_frames(data: &[u8]) -> u32 {
    const MAGIC: [u8; 8] = [0x89, b'B', b'I', b'F', 0x0d, 0x0a, 0x1a, 0x0a];
    if data.len() < 16 || data[..8] != MAGIC {
        return 0;
    }
    u32::from_le_bytes([data[12], data[13], data[14], data[15]])
}

/// The Emby markers of `item` as `<stem>.jellyfin.json` content, `None`
/// without markers.
pub(crate) fn emby_segments_file(item: &Item) -> Option<Vec<u8>> {
    let s = segments_from_chapters(&item.chapters, item.run_time_ticks.map(ticks_to_ms));
    (!s.is_empty()).then(|| segments_json(&item.id, &s))
}

pub(crate) fn image_path(
    item_id: &str,
    image_type: &str,
    tag: Option<&str>,
    max_width: u32,
) -> String {
    let mut p = format!(
        "/Items/{}/Images/{image_type}?maxWidth={max_width}&quality=90&format=Jpg",
        encode(item_id)
    );
    if let Some(t) = tag {
        p.push_str(&format!("&tag={}", encode(t)));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bif_frames() {
        let mut empty = vec![0x89, b'B', b'I', b'F', 0x0d, 0x0a, 0x1a, 0x0a];
        empty.extend([0u8; 56]);
        assert_eq!(bif_frames(&empty), 0);
        empty[12] = 3;
        assert_eq!(bif_frames(&empty), 3);
        assert_eq!(bif_frames(b"not a bif at all"), 0);
        assert_eq!(bif_path("1 2", 320), "/Videos/1%202/index.bif?Width=320");
    }

    #[test]
    fn emby_quick_connect_is_unsupported_without_a_request() {
        // Port 9 is closed: any request would fail as unreachable.
        let c = Client::new(
            &["http://127.0.0.1:9".into()],
            crate::DeviceInfo::kinolune("d", "i"),
        )
        .unwrap()
        .with_kind(ServerKind::Emby);
        assert_eq!(c.quick_connect_enabled(), Ok(false));
        assert!(matches!(c.quick_connect_start(), Err(Error::Unsupported(_))));
        assert!(matches!(c.media_segments("x"), Err(Error::Unsupported(_))));
        assert!(matches!(c.me(), Err(Error::Invalid(_))));
    }
}
