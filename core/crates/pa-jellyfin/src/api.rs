// SPDX-License-Identifier: MPL-2.0
//! Endpoints: server info, sign-in, users, libraries, items, Continue
//! Watching, favorites, played state, media segments and trickplay.
//!
//! Paths are those of Jellyfin 10.9+ (`/UserViews`, `/UserItems/Resume`,
//! `/UserFavoriteItems/{id}`, `/UserItems/{id}/UserData`, POST
//! `/QuickConnect/Initiate`); the integration tests pin 10.10.7.

use serde_json::json;

use crate::http::{Client, Method, Token, encode};
use crate::model::{
    AuthenticationResult, Item, ItemPage, PublicSystemInfo, QuickConnectState, RawSegments,
    Segment, TrickplayInfo, User,
};
use crate::{Error, Result};

/// Fields every item listing asks for: what the virtual tree, the
/// playback plan and the user data import need.
pub const ITEM_FIELDS: &str =
    "MediaSources,ProviderIds,Path,DateCreated,Trickplay,ParentId,Overview";

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
        Ok(s)
    }

    /// Whether Quick Connect is switched on at the server.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn quick_connect_enabled(&self) -> Result<bool> {
        self.get_json("/QuickConnect/Enabled")
    }

    /// Starts Quick Connect: show [`QuickConnectState::code`] to the user,
    /// who approves it in a signed-in client; then poll
    /// [`quick_connect_state`](Self::quick_connect_state) and finish with
    /// [`sign_in_quick_connect`](Self::sign_in_quick_connect).
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] if Quick Connect is switched off.
    pub fn quick_connect_start(&self) -> Result<QuickConnectState> {
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
        self.get_json(&format!("/QuickConnect/Connect?secret={}", encode(secret)))
    }

    /// Trades an approved Quick Connect secret for a token and keeps it.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] if the code has not been approved.
    pub fn sign_in_quick_connect(&self, secret: &str) -> Result<Session> {
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
        r.map(|_| ())
    }

    /// The signed-in user.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] if the token is no longer valid.
    pub fn me(&self) -> Result<User> {
        self.get_json("/Users/Me")
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
        let page: ItemPage = self.get_json(&format!("/UserViews?userId={}", encode(user_id)))?;
        Ok(page.items)
    }

    /// One item as `user_id` sees it, with [`ITEM_FIELDS`].
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown id.
    pub fn item(&self, user_id: &str, item_id: &str) -> Result<Item> {
        self.get_json(&format!(
            "/Items/{}?userId={}&fields={ITEM_FIELDS}",
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
        let mut base = format!(
            "/Items?userId={}&fields={ITEM_FIELDS}&enableUserData=true\
             &enableImageTypes=Primary,Backdrop&imageTypeLimit=1&sortBy=SortName",
            encode(user_id)
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
        let page: ItemPage = self.get_json(&format!(
            "/UserItems/Resume?userId={}&mediaTypes=Video&fields={ITEM_FIELDS}\
             &enableUserData=true&limit={limit}",
            encode(user_id)
        ))?;
        Ok(page.items)
    }

    /// Removes `item_id` from Continue Watching: drops the resume position
    /// and leaves "watched" and the play count as they are. This is what
    /// Jellyfin's own clients do; the server has no separate hide flag.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn remove_from_resume(&self, user_id: &str, item_id: &str) -> Result<()> {
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
        let path = format!(
            "/UserFavoriteItems/{}?userId={}",
            encode(item_id),
            encode(user_id)
        );
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
        let path = format!(
            "/UserPlayedItems/{}?userId={}",
            encode(item_id),
            encode(user_id)
        );
        let method = if played { Method::Post } else { Method::Delete };
        self.call(method, &path, None)?;
        Ok(())
    }

    /// Media segments of `item_id` (intro, credits, …; Jellyfin 10.10+
    /// with a segment provider). Empty on older servers.
    ///
    /// # Errors
    ///
    /// Transport errors.
    pub fn media_segments(&self, item_id: &str) -> Result<Vec<Segment>> {
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
    /// [`Error::NotFound`] on servers without the endpoint.
    pub fn media_segments_json(&self, item_id: &str) -> Result<Vec<u8>> {
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
