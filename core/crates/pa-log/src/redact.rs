// SPDX-License-Identifier: MPL-2.0
//! Redaction of user paths and secrets (CORE-008, ADR-NNNN).
//!
//! Every line goes through [`Redactor::redact`] before it reaches a log
//! file, a crash report or the diagnostic report. The rules are
//! deliberately greedy: a lost hex id in a log costs less than a leaked
//! token in a public GitHub issue.
//!
//! What is cut:
//! - home directories: the known ones (`$HOME`, `%USERPROFILE%`, the ones
//!   the app passes) become `~`; any other `/home/<user>`, `/Users/<user>`,
//!   `C:\Users\<user>` keeps the prefix and loses the user name; the iOS app
//!   container UUID is cut;
//! - user info of URLs (`smb://user:password@host`), also percent-encoded;
//! - query parameters that carry secrets (`access_token`, `api_key`,
//!   `X-Plex-Token`, Subsonic `u`/`p`/`t`/`s`, Twitch usher `sig`/`token`…);
//! - `key = value` / `"key": "value"` pairs with a secret-looking key
//!   (password, token, secret, cookie…);
//! - `Authorization` schemes (`Bearer`, `OAuth`, `Basic`…), IRC `oauth:…`;
//! - the pa-proxy path token (`http://127.0.0.1:<port>/<token>/…`);
//! - JWTs and any hex run of 32+ digits (TMDB keys, Jellyfin tokens);
//! - literal secrets registered with [`Redactor::add_secret`].

use std::borrow::Cow;

use regex::Regex;

/// Replacement for a cut secret.
pub const REDACTED: &str = "<redacted>";

struct Rule {
    re: Regex,
    with: &'static str,
}

fn rule(pattern: &str, with: &'static str) -> Rule {
    Rule {
        re: Regex::new(pattern).expect("valid redaction pattern"),
        with,
    }
}

/// Query and form parameters whose value is a secret or a user name.
const SECRET_PARAMS: &str = "access_token|refresh_token|id_token|oauth_token|token|auth|authorization|\
client_secret|secret|password|passwd|pass|pwd|api_key|apikey|api-key|key|sig|signature|\
x-plex-token|x-emby-token|x-mediabrowser-token|code|session|sessionid|sid|nauth|nauthsig|\
u|p|t|s|username|user";

/// Keys of `key = value` pairs whose value is a secret.
const SECRET_KEYS: &str = "password|passwd|pwd|secret|client_secret|token|access_token|refresh_token|\
id_token|api_key|apikey|authorization|x-emby-token|x-plex-token|x-mediabrowser-token|cookie|\
set-cookie|pin|session|sessionid";

/// Cuts user paths and secrets out of text. Cheap to clone the rules once;
/// one instance is shared by the logger, the crash hook and the report.
pub struct Redactor {
    homes: Vec<String>,
    secrets: Vec<String>,
    rules: Vec<Rule>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Neither homes nor secrets are printed.
        f.debug_struct("Redactor")
            .field("homes", &self.homes.len())
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

impl Default for Redactor {
    fn default() -> Self {
        Self::with_homes(Vec::<String>::new())
    }
}

impl Redactor {
    /// Rules plus the home directories of this process (`HOME`,
    /// `USERPROFILE`) and [`extra_homes`](Self::new) (the app's own view of
    /// them, e.g. from `path_provider`).
    pub fn new(extra_homes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let env = ["HOME", "USERPROFILE"]
            .iter()
            .filter_map(|v| std::env::var(v).ok());
        Self::with_homes(env.chain(extra_homes.into_iter().map(Into::into)))
    }

    /// Rules plus exactly these home directories (tests, or a caller that
    /// knows better than the environment).
    pub fn with_homes(homes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut homes: Vec<String> = homes
            .into_iter()
            .map(Into::into)
            .map(|h| h.trim_end_matches(['/', '\\']).to_string())
            // `/` or `C:` as a "home" would eat every path.
            .filter(|h| h.len() > 3)
            .collect();
        // Longest first: `/home/a/b` before `/home/a`.
        homes.sort_by_key(|h| std::cmp::Reverse(h.len()));
        homes.dedup();
        let rules = vec![
            // URL user info, plain and percent-encoded (an SMB URL inside a
            // proxy URL).
            rule(
                r"(?i)\b([a-z][a-z0-9+.\-]*://)[^\s/?#@]+@",
                "${1}<redacted>@",
            ),
            rule(
                r"(?i)(%3A%2F%2F)[^\s/?&#%]*(?:%(?:[0-9a-f]{2})[^\s/?&#%]*)*?%40",
                "${1}<redacted>%40",
            ),
            // Secret query parameters.
            rule(
                &format!(r#"(?i)([?&#;](?:{SECRET_PARAMS})=)[^&#\s"'<>]*"#),
                "${1}<redacted>",
            ),
            // HTTP authorization schemes and IRC PASS.
            rule(
                r"(?i)\b(Bearer|OAuth|Basic|Digest|Token|MediaBrowser)\s+[A-Za-z0-9._~+/=\-]{6,}",
                "${1} <redacted>",
            ),
            rule(r"(?i)\boauth:[A-Za-z0-9]+", "oauth:<redacted>"),
            // `key: value`, `key=value`, `"key": "value"`.
            rule(
                &format!(
                    r#"(?i)(["']?\b(?:{SECRET_KEYS})["']?\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;}}&]+)"#
                ),
                "${1}<redacted>",
            ),
            // The pa-proxy token is the first path segment of a loopback URL.
            rule(
                r"(?i)(https?://(?:127\.0\.0\.1|localhost|\[::1\])(?::\d+)?/)[^\s/?#]+",
                "${1}<proxy-token>",
            ),
            // JWTs and long hex runs (API keys, access tokens).
            rule(
                r"\beyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]*",
                "<jwt>",
            ),
            rule(r"\b[0-9A-Fa-f]{32,}\b", "<hex>"),
            // Home directories nobody told us about.
            rule(r"(/home/)[^/\s\x22'<>]+", "${1}<user>"),
            rule(r"(/Users/)[^/\s\x22'<>]+", "${1}<user>"),
            rule(
                r"(?i)\b([a-z]:[\\/]+Users[\\/]+)[^\\/\s\x22'<>]+",
                "${1}<user>",
            ),
            rule(r"(?i)(%2Fhome%2F|%2FUsers%2F)[^%\s/]+", "${1}<user>"),
            rule(
                r"(?i)(/Containers/(?:Data|Shared)/(?:Application|AppGroup)/)[0-9A-F\-]{36}",
                "${1}<app>",
            ),
        ];
        Self {
            homes,
            secrets: Vec::new(),
            rules,
        }
    }

    /// Adds a literal secret (a token the caller holds) to cut wherever it
    /// shows up. Values shorter than 6 characters are ignored: they would
    /// cut ordinary words.
    pub fn add_secret(&mut self, secret: impl Into<String>) {
        let s = secret.into();
        if s.len() >= 6 && !self.secrets.contains(&s) {
            self.secrets.push(s);
            self.secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        }
    }

    /// `text` without user paths and secrets.
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut out = Cow::Borrowed(text);
        for s in &self.secrets {
            if out.contains(s.as_str()) {
                out = Cow::Owned(out.replace(s.as_str(), REDACTED));
            }
        }
        for h in &self.homes {
            for variant in home_variants(h) {
                if let Some(s) = replace_path(&out, &variant, "~") {
                    out = Cow::Owned(s);
                }
            }
        }
        for r in &self.rules {
            let replaced = match r.re.replace_all(&out, r.with) {
                Cow::Owned(s) => Some(s),
                Cow::Borrowed(_) => None,
            };
            if let Some(s) = replaced {
                out = Cow::Owned(s);
            }
        }
        out
    }

    /// Whether a configuration key names a secret; its value is not
    /// exported at all.
    pub fn is_secret_key(key: &str) -> bool {
        let k = key.to_ascii_lowercase();
        [
            "pass", "secret", "token", "key", "auth", "cookie", "pin", "session", "credential",
        ]
        .iter()
        .any(|w| k.contains(w))
            // `…_max_bytes` and friends are not secrets.
            && !k.ends_with("_bytes")
    }
}

/// `text` with every occurrence of the path `prefix` that ends at a path
/// boundary (`/home/al` must not eat `/home/alice`) replaced; `None` when
/// there is none.
fn replace_path(text: &str, prefix: &str, with: &str) -> Option<String> {
    let mut out = String::new();
    let mut last = 0;
    let mut hit = false;
    for (i, _) in text.match_indices(prefix) {
        if i < last {
            continue;
        }
        let end = i + prefix.len();
        let boundary = text[end..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')));
        if boundary {
            out.push_str(&text[last..i]);
            out.push_str(with);
            last = end;
            hit = true;
        }
    }
    hit.then(|| {
        out.push_str(&text[last..]);
        out
    })
}

/// A home as written in logs: as is, with the other separator, and
/// percent-encoded (inside URLs).
fn home_variants(h: &str) -> Vec<String> {
    let mut v = vec![h.to_string()];
    if h.contains('\\') {
        v.push(h.replace('\\', "/"));
    }
    let encoded = h.replace('/', "%2F").replace('\\', "%5C");
    if encoded != h {
        v.push(encoded);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Redactor {
        Redactor::with_homes(["/home/alice", "C:\\Users\\Alice"])
    }

    #[test]
    fn known_homes_become_tilde() {
        let r = r();
        assert_eq!(
            r.redact("open /home/alice/Videos/a.mkv"),
            "open ~/Videos/a.mkv"
        );
        assert_eq!(r.redact(r"C:\Users\Alice\Videos"), r"~\Videos");
        assert_eq!(r.redact("C:/Users/Alice/Videos"), "~/Videos");
        assert_eq!(
            r.redact("file%3A%2F%2F%2Fhome%2Falice%2Fx"),
            "file%3A%2F%2F~%2Fx"
        );
    }

    #[test]
    fn unknown_homes_lose_the_user() {
        let r = Redactor::default();
        assert!(r.redact("/home/bob/x").contains("/home/<user>/x"));
        assert!(r.redact("/Users/bob/x").contains("/Users/<user>/x"));
        assert!(r.redact(r"D:\Users\bob\x").contains(r"D:\Users\<user>\x"));
        let ios =
            "/var/mobile/Containers/Data/Application/0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0/Library";
        assert_eq!(
            r.redact(ios),
            "/var/mobile/Containers/Data/Application/<app>/Library"
        );
    }

    #[test]
    fn url_tokens_and_user_info() {
        let r = Redactor::default();
        let s = r.redact("GET https://jf.lan/Items?api_key=abc123&Fields=x");
        assert_eq!(s, "GET https://jf.lan/Items?api_key=<redacted>&Fields=x");
        let s = r.redact("smb://bob:hunter2@nas/share/a.mkv");
        assert_eq!(s, "smb://<redacted>@nas/share/a.mkv");
        let s = r.redact("rest/ping?u=bob&t=26719a1196d2a940705a59634eb18eab&s=c19b2d&v=1.16");
        assert!(
            !s.contains("bob") && !s.contains("26719a") && !s.contains("c19b2d"),
            "{s}"
        );
        let s = r.redact("usher/vod.m3u8?sig=deadbeef&token=%7B%22a%22%7D&allow_source=true");
        assert!(!s.contains("deadbeef") && !s.contains("%7B"), "{s}");
        assert!(s.contains("allow_source=true"));
        let s = r.redact("http://127.0.0.1:41234/3f0c9a1b2c3d4e5f60718293a4b5c6d7/smb%3A%2F%2Fbob%3Apw%40nas%2Fa");
        assert!(
            s.starts_with("http://127.0.0.1:41234/<proxy-token>/"),
            "{s}"
        );
        assert!(!s.contains("bob") && !s.contains("pw%40"), "{s}");
    }

    #[test]
    fn headers_and_pairs() {
        let r = Redactor::default();
        let s = r.redact("Authorization: Bearer abcdefghijklmnop");
        assert!(
            s.starts_with("Authorization: ") && !s.contains("abcdefghijklmnop"),
            "{s}"
        );
        assert_eq!(
            r.redact("header OAuth 0123456789abcdefghijklmnopqrst"),
            "header OAuth <redacted>"
        );
        assert_eq!(r.redact("PASS oauth:abcdef123456"), "PASS oauth:<redacted>");
        assert_eq!(
            r.redact(r#"{"password": "hunter2", "name": "x"}"#),
            r#"{"password": <redacted>, "name": "x"}"#
        );
        assert_eq!(
            r.redact("client_secret=s3cr3t done"),
            "client_secret=<redacted> done"
        );
        assert!(
            !r.redact("X-Emby-Token: 0123abcd0123abcd")
                .contains("0123abcd")
        );
        assert_eq!(
            r.redact("jwt eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl"),
            "jwt <jwt>"
        );
        assert_eq!(
            r.redact("tmdb 0123456789abcdef0123456789abcdef"),
            "tmdb <hex>"
        );
    }

    #[test]
    fn home_prefix_stops_at_a_boundary() {
        let r = Redactor::with_homes(["/home/al"]);
        assert_eq!(r.redact("/home/al/x /home/alice/y"), "~/x /home/<user>/y");
    }

    #[test]
    fn registered_secrets() {
        let mut r = Redactor::default();
        r.add_secret("tw1tchT0kenXyz");
        r.add_secret("abc"); // too short: ignored
        assert_eq!(
            r.redact("got tw1tchT0kenXyz back, abc"),
            "got <redacted> back, abc"
        );
    }

    #[test]
    fn plain_text_is_untouched() {
        let r = Redactor::default();
        let s = "scan finished: 12 files, 3 new in 140 ms";
        assert!(matches!(r.redact(s), Cow::Borrowed(_)));
    }

    #[test]
    fn secret_keys() {
        assert!(Redactor::is_secret_key("tmdb_api_key"));
        assert!(Redactor::is_secret_key("profiles_pin_attempts"));
        assert!(!Redactor::is_secret_key("image_cache_max_bytes"));
        assert!(!Redactor::is_secret_key("ui_locale"));
    }
}
