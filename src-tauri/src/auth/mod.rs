//! Google authentication. Nothing here crosses the IPC boundary except the
//! connected/disconnected fact — see ARCHITECTURE §3 and §7.

pub mod client_config;
pub mod flow;
pub mod loopback;
pub mod pkce;
pub mod tokens;

use std::sync::Mutex;

use tokens::TokenCache;

pub use tokens::AccessToken;

/// Guards against two sign-ins racing: a second click while a browser tab is
/// already open would bind a second listener and leave one orphaned.
#[derive(Default)]
pub struct AuthState {
    pub cache: TokenCache,
    pub in_progress: Mutex<bool>,
}

impl AuthState {
    /// Returns false if a sign-in is already running.
    pub fn begin(&self) -> bool {
        match self.in_progress.lock() {
            Ok(mut guard) if !*guard => {
                *guard = true;
                true
            }
            _ => false,
        }
    }

    pub fn finish(&self) {
        if let Ok(mut guard) = self.in_progress.lock() {
            *guard = false;
        }
    }

    /// A usable access token, refreshing if the cached one is near expiry.
    ///
    /// The only route to a token for API calls. Callers get a `String` they
    /// attach to one request and drop — nothing caches it further, and nothing
    /// hands it toward the WebView.
    pub fn access_token(&self) -> Result<String, AuthError> {
        if let Some(token) = self.cache.valid_token() {
            return Ok(token);
        }

        let Some(refresh_token) = tokens::load_refresh_token()
            .map_err(AuthError::Storage)?
        else {
            return Err(AuthError::NotConnected);
        };

        match flow::refresh_access_token(&refresh_token) {
            Ok(access) => {
                let value = access.expose().to_string();
                self.cache.store(access);
                Ok(value)
            }
            Err(message) => {
                // A refresh token can die on us: revoked from the Google account
                // page, six months unused, or pushed out by the 100-token cap.
                // All of them mean the same thing — sign in again.
                log::warn!("access token refresh failed: {message}");
                Err(AuthError::RefreshFailed(message))
            }
        }
    }
}

#[derive(Debug)]
pub enum AuthError {
    NotConnected,
    RefreshFailed(String),
    Storage(String),
}

impl AuthError {
    pub fn user_message(&self) -> String {
        match self {
            AuthError::NotConnected => "Connect Google to see your tasks.".into(),
            AuthError::RefreshFailed(_) => {
                "Google access expired. Reconnect in Settings.".into()
            }
            AuthError::Storage(_) => {
                "Could not read your saved Google sign-in. Reconnect in Settings."
                    .into()
            }
        }
    }
}
