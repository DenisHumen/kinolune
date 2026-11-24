// SPDX-License-Identifier: MPL-2.0
//! Trakt API v2 client (SYNC-011, ADR-0154).
//!
//! - **Sign-in** by the device code flow ([`Session::begin`],
//!   [`Session::poll`]): the app shows a short code and a QR code of the
//!   activation page, so it works on a TV. Tokens live in the secret store
//!   ([`pa_secrets::SecretKey::trakt_tokens`]) and are refreshed before they
//!   expire; refresh tokens are single-use, the new pair replaces the old
//!   one at once.
//! - **Scrobble** start/pause/stop of what the player plays
//!   ([`Session::scrobble`]), by the TMDB/IMDb/TVDB ids of the title.
//! - **Watched history**, **watchlist** and **ratings** (`/sync/*`), and
//!   the two-way sync of watched marks and ratings with the library
//!   ([`sync`]).
//!
//! Credentials: the project's `client_id` is compiled in from
//! `PA_TRAKT_CLIENT_ID` / `CARGO_PA_TRAKT_CLIENT_ID` (like the TMDB key);
//! the user may enter an own client id. Trakt marks `client_secret`
//! deprecated for user sign-in and optional in the device flow: it is sent
//! only if one is configured (`PA_TRAKT_CLIENT_SECRET`, or the user's own).
//! A secret shipped inside an open-source app is not secret anyway.
//!
//! Rate limits (Standard apps): writes at most once a second per user
//! (enforced here), `GET` 500 per 5 minutes; a `429` is answered with its
//! `Retry-After`. Everything blocks on the network: call it off the UI
//! thread. No live network in tests: [`testing::Recorded`] replays
//! recorded answers.

mod auth;
mod client;
mod error;
mod http;
mod model;
mod session;
pub mod sync;
pub mod testing;
mod time;

pub use auth::{Credentials, DeviceCode, Poll, Tokens};
pub use client::{Client, Endpoints, ScrobbleAction};
pub use error::{Error, Result};
pub use http::{Method, Request, Response, Transport, UreqTransport};
pub use model::{Ids, Listed, Rated, Target, Watched, stars_from_trakt, stars_to_trakt};
pub use session::{ACCOUNT_ID, Session};
pub use time::{format_ms, parse_ms};
