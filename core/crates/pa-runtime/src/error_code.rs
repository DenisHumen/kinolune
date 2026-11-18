// SPDX-License-Identifier: MPL-2.0
//! Stable error codes (CORE-009, ADR-0120).
//!
//! Every error a facade returns to the app carries a code: a short
//! `snake_case` ASCII word such as `not_found` or `unreachable`. The app
//! turns the code into text with its own l10n (`message` is a detail for
//! logs, in English, never shown as is). Codes are part of the API: a
//! released code keeps its meaning; new codes may be added, so the app
//! needs a fallback text for codes it does not know.
//!
//! Domains keep their own specific codes (`usher_403`, `store_build`);
//! the words in [`common`] mean the same in every domain, so one l10n
//! string serves all of them.

/// An error with a stable code.
pub trait ErrorCode {
    /// The code of this error; always passes [`is_valid_code`].
    fn code(&self) -> &'static str;
}

/// Codes shared by all domains. A domain uses these words for these
/// situations rather than inventing synonyms.
pub mod common {
    /// No such item, source, file, device or key.
    pub const NOT_FOUND: &str = "not_found";
    /// An argument was not acceptable (bad id, bad key, bad URL).
    pub const INVALID: &str = "invalid";
    /// The active profile or the user is not allowed to do this.
    pub const DENIED: &str = "denied";
    /// No network, or the service did not answer.
    pub const NETWORK: &str = "network";
    /// A source or server is configured but cannot be reached now (share
    /// offline, wrong credentials).
    pub const UNREACHABLE: &str = "unreachable";
    /// Reading or writing a local file failed.
    pub const IO: &str = "io";
    /// A local database failed (locked, corrupt, full).
    pub const DATABASE: &str = "database";
    /// The secret store (keychain) failed.
    pub const SECRETS: &str = "secrets";
    /// Not enough free space.
    pub const NO_SPACE: &str = "no_space";
    /// Something else holds the resource; try again later.
    pub const BUSY: &str = "busy";
    /// The feature is off (build flavor, remote config, setting).
    pub const DISABLED: &str = "disabled";
    /// The core or the object was not started or is closed already.
    pub const NOT_INITIALIZED: &str = "not_initialized";
    /// Sync of the device family failed.
    pub const SYNC: &str = "sync";
    /// A metadata provider failed or none is set up.
    pub const PROVIDER: &str = "provider";
    /// A bug or an unexpected state; details are in the log.
    pub const INTERNAL: &str = "internal";

    /// All common codes.
    pub const ALL: &[&str] = &[
        NOT_FOUND,
        INVALID,
        DENIED,
        NETWORK,
        UNREACHABLE,
        IO,
        DATABASE,
        SECRETS,
        NO_SPACE,
        BUSY,
        DISABLED,
        NOT_INITIALIZED,
        SYNC,
        PROVIDER,
        INTERNAL,
    ];
}

/// Longest allowed code.
pub const MAX_CODE_LEN: usize = 40;

/// Whether `code` has the form of an error code: `[a-z][a-z0-9_]*`, at
/// most [`MAX_CODE_LEN`] bytes, no double or trailing `_`.
#[must_use]
pub fn is_valid_code(code: &str) -> bool {
    let bytes = code.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_CODE_LEN
        && bytes[0].is_ascii_lowercase()
        && !code.ends_with('_')
        && !code.contains("__")
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_codes_are_valid_and_unique() {
        for code in common::ALL {
            assert!(is_valid_code(code), "{code}");
        }
        let mut sorted = common::ALL.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), common::ALL.len());
    }

    #[test]
    fn code_shape() {
        assert!(is_valid_code("usher_403"));
        assert!(is_valid_code("io"));
        for bad in [
            "",
            "Io",
            "1x",
            "_x",
            "x_",
            "a__b",
            "a-b",
            "a.b",
            &"a".repeat(41),
        ] {
            assert!(!is_valid_code(bad), "{bad}");
        }
    }
}
