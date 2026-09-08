//! Token storage.
//!
//! ARCHITECTURE §7.1, without exception:
//!   - the refresh token lives only in Windows Credential Manager
//!   - the access token lives only in process memory
//!   - neither is ever logged, serialized to disk, or sent over IPC
//!
//! Nothing in this module has a Debug impl that could print a token, and nothing
//! returns one to the caller except the HTTP layer that must attach it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use keyring::Entry;

const SERVICE: &str = "in.startupmission.stickywidget";
const ACCOUNT: &str = "google-refresh-token";

/// Refresh this far before actual expiry, so a request never races the clock.
const REFRESH_MARGIN: Duration = Duration::from_secs(300);

pub struct AccessToken {
    value: String,
    expires_at: Instant,
}

impl AccessToken {
    pub fn new(value: String, expires_in_secs: u64) -> Self {
        Self {
            value,
            expires_at: Instant::now() + Duration::from_secs(expires_in_secs),
        }
    }

    pub fn needs_refresh(&self) -> bool {
        Instant::now() + REFRESH_MARGIN >= self.expires_at
    }

    /// The only way out. Callers attach it to a request and drop it.
    pub fn expose(&self) -> &str {
        &self.value
    }
}

/// In-memory only, by construction.
#[derive(Default)]
pub struct TokenCache {
    access: Mutex<Option<AccessToken>>,
}

impl TokenCache {
    pub fn store(&self, token: AccessToken) {
        if let Ok(mut guard) = self.access.lock() {
            *guard = Some(token);
        }
    }

    pub fn valid_token(&self) -> Option<String> {
        let guard = self.access.lock().ok()?;
        let token = guard.as_ref()?;
        if token.needs_refresh() {
            None
        } else {
            Some(token.expose().to_string())
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.access.lock() {
            *guard = None;
        }
    }
}

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| format!("Windows Credential Manager is unavailable: {e}"))
}

pub fn save_refresh_token(token: &str) -> Result<(), String> {
    entry()?
        .set_password(token)
        .map_err(|e| format!("Could not save your Google sign-in: {e}"))
}

pub fn load_refresh_token() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(format!("Could not read your saved Google sign-in: {err}")),
    }
}

/// Idempotent: a missing entry is success, since the goal is "not stored".
pub fn delete_refresh_token() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(format!("Could not remove your saved Google sign-in: {err}")),
    }
}

pub fn has_refresh_token() -> bool {
    matches!(load_refresh_token(), Ok(Some(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_token_wants_refresh_inside_the_margin() {
        let fresh = AccessToken::new("t".into(), 3600);
        assert!(!fresh.needs_refresh());

        // Expiry inside the 5-minute margin must count as due.
        let soon = AccessToken::new("t".into(), 60);
        assert!(soon.needs_refresh());

        let expired = AccessToken::new("t".into(), 0);
        assert!(expired.needs_refresh());
    }

    #[test]
    fn cache_withholds_a_token_that_is_due_for_refresh() {
        let cache = TokenCache::default();
        assert_eq!(cache.valid_token(), None);

        cache.store(AccessToken::new("good".into(), 3600));
        assert_eq!(cache.valid_token().as_deref(), Some("good"));

        cache.store(AccessToken::new("stale".into(), 10));
        assert_eq!(cache.valid_token(), None, "near-expiry token must not be served");

        cache.store(AccessToken::new("good".into(), 3600));
        cache.clear();
        assert_eq!(cache.valid_token(), None);
    }

    /// Touches the real Credential Manager, so it is opt-in:
    /// `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn refresh_token_round_trips_through_credential_manager() {
        let secret = "test-refresh-token-value";
        save_refresh_token(secret).expect("save");
        assert_eq!(load_refresh_token().unwrap().as_deref(), Some(secret));

        delete_refresh_token().expect("delete");
        assert_eq!(load_refresh_token().unwrap(), None);

        // Deleting again must not error.
        delete_refresh_token().expect("second delete should be a no-op");
    }
}
