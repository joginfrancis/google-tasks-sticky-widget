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
            // A refresh token can die on us: revoked from the Google account
            // page, six months unused, or pushed out by the 100-token cap.
            // Those mean sign in again.
            Err(err) if err.is_rejection() => {
                log::warn!("google refused the refresh token: {}", err.message());
                Err(AuthError::RefreshFailed(err.into_message()))
            }
            // Not reaching Google is weather, not a verdict. It used to be
            // treated as the same thing, which is how a sleeping laptop lost
            // its session while the credential sat valid on disk.
            Err(err) => {
                log::warn!("could not refresh the access token: {}", err.message());
                Err(AuthError::Offline(err.into_message()))
            }
        }
    }

    /// Whether a 401 from the Tasks API really means the sign-in is gone.
    ///
    /// A 401 on its own does not: the cached access token may simply have gone
    /// stale while the machine slept. The grant is dead only if Google refuses
    /// to mint a new token from it - and if Google cannot be reached to ask,
    /// the honest answer is "don't know", which must not cost a session.
    ///
    /// Refreshes on the way past, so a recovered token is kept rather than
    /// thrown away and asked for again.
    pub fn refresh_token_is_dead(&self) -> bool {
        self.cache.clear();

        match tokens::load_refresh_token() {
            Ok(Some(refresh)) => match flow::refresh_access_token(&refresh) {
                Ok(access) => {
                    self.cache.store(access);
                    false
                }
                Err(err) => {
                    if !err.is_rejection() {
                        log::warn!(
                            "could not check the sign-in after a 401: {}",
                            err.message()
                        );
                    }
                    err.is_rejection()
                }
            },
            // Nothing stored: genuinely not connected.
            Ok(None) => true,
            // The credential store is unreadable. That is not evidence that
            // the user signed out, and treating it as such would clear a
            // session over a transient Credential Manager hiccup.
            Err(err) => {
                log::warn!("could not read the saved sign-in: {err}");
                false
            }
        }
    }
}

#[derive(Debug)]
pub enum AuthError {
    NotConnected,
    RefreshFailed(String),
    /// Google could not be reached. The sign-in is intact.
    Offline(String),
    Storage(String),
}

impl AuthError {
    pub fn user_message(&self) -> String {
        match self {
            AuthError::NotConnected => "Connect Google to see your tasks.".into(),
            AuthError::RefreshFailed(_) => {
                "Google access expired. Reconnect in Settings.".into()
            }
            AuthError::Offline(_) => {
                "Can't reach Google. Your tasks will sync when the connection is back."
                    .into()
            }
            AuthError::Storage(_) => {
                "Could not read your saved Google sign-in. Reconnect in Settings."
                    .into()
            }
        }
    }
}
