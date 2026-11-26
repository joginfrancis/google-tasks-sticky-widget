//! The authorization code + PKCE flow, and token refresh.

use serde::Deserialize;

use super::client_config::ClientConfig;
use super::loopback::{CallbackResult, Loopback};
use super::pkce::Pkce;
use super::tokens::{self, AccessToken};

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";

/// SPEC §2.1 — one scope. Not Gmail, Drive, Calendar, or Contacts.
const SCOPE: &str = "https://www.googleapis.com/auth/tasks";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
}

pub struct Authenticated {
    pub access: AccessToken,
    pub refresh_token: Option<String>,
}

fn build_auth_url(config: &ClientConfig, pkce: &Pkce, redirect_uri: &str) -> String {
    let enc = urlencoding::encode;
    format!(
        "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code\
         &scope={}&code_challenge={}&code_challenge_method=S256&state={}\
         &access_type=offline&prompt=consent",
        enc(&config.client_id),
        enc(redirect_uri),
        enc(SCOPE),
        enc(&pkce.challenge),
        enc(&pkce.state),
    )
}

/// Runs the whole interactive flow. Blocking — callers put it on a background
/// thread so the UI keeps painting.
///
/// `open_browser` is injected rather than called directly so the flow can be
/// exercised without launching anything.
pub fn authorize<F>(open_browser: F) -> Result<Authenticated, String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let config = ClientConfig::load()?;
    let pkce = Pkce::generate()?;
    let loopback = Loopback::bind()?;

    // Captured before the listener is consumed by the wait below.
    let redirect_uri = loopback.redirect_uri.clone();

    let url = build_auth_url(&config, &pkce, &redirect_uri);
    // Never log `url` — it carries the state and challenge (ARCHITECTURE §7.3).
    log::info!("opening browser for Google sign-in");
    open_browser(&url)?;

    let callback = loopback.wait_for_callback()?;

    let (code, state) = match callback {
        CallbackResult::Success { code, state } => (code, state),
        CallbackResult::Denied { error, .. } => {
            log::info!("sign-in did not complete: {error}");
            return Err(match error.as_str() {
                "access_denied" => "Sign-in was cancelled.".into(),
                "missing_state" => {
                    "Sign-in response was incomplete. Please try again.".into()
                }
                other => format!("Google declined the sign-in ({other})."),
            });
        }
    };

    if !pkce.state_matches(&state) {
        // Either a stale tab or something local trying to inject a code.
        log::warn!("rejected callback: state mismatch");
        return Err("Sign-in could not be verified. Please try again.".into());
    }

    let response = exchange_code(&config, &code, &pkce.verifier, &redirect_uri)
        .map_err(|err| err.into_message())?;

    let refresh_token = response.refresh_token.clone();
    if let Some(token) = &refresh_token {
        tokens::save_refresh_token(token)?;
    } else {
        // access_type=offline + prompt=consent should always yield one.
        log::warn!("Google returned no refresh token; sign-in will not persist");
    }

    Ok(Authenticated {
        access: AccessToken::new(response.access_token, response.expires_in),
        refresh_token,
    })
}

/// Sends the client secret when we have one.
///
/// Measured 2026-09-07: Google's docs list `client_secret` as optional, but a
/// real Desktop client rejects the exchange without it (`400 invalid_request`).
/// So the secret goes on the first attempt rather than after a wasted round
/// trip. It ships inside the binary and is not confidential — PKCE is what
/// secures this flow. See ARCHITECTURE §4.2.
///
/// The no-secret path is kept for the case where a credentials file genuinely
/// carries no secret; it is not a fallback we expect to exercise.
fn token_form(config: &ClientConfig, mut form: Vec<(&'static str, String)>) -> Vec<(&'static str, String)> {
    form.push(("client_id", config.client_id.clone()));
    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    form
}

fn exchange_code(
    config: &ClientConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, TokenError> {
    post_token(token_form(
        config,
        vec![
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
            ("grant_type", "authorization_code".to_string()),
            ("code_verifier", verifier.to_string()),
        ],
    ))
}

/// Why a call to Google's token endpoint failed.
///
/// The distinction that matters is whether Google answered at all. A request
/// that never left the machine says nothing about the grant, and treating it
/// as a dead sign-in is what put the connect screen in front of a refresh
/// token sitting perfectly valid in Credential Manager - every time a laptop
/// woke up before its network did.
#[derive(Debug)]
pub enum TokenError {
    /// Never reached Google: no route, DNS, TLS, timeout. Says nothing about
    /// the grant, so the only correct response is to try again later.
    Unreachable(String),
    /// Google answered and refused the grant. Signing in again is the only
    /// way back.
    Rejected(String),
    /// Google answered with something unusable - a 5xx, or a body that did
    /// not parse. Not evidence against the grant either.
    Unexpected(String),
}

impl TokenError {
    pub fn message(&self) -> &str {
        match self {
            TokenError::Unreachable(m) | TokenError::Rejected(m) | TokenError::Unexpected(m) => m,
        }
    }

    pub fn into_message(self) -> String {
        match self {
            TokenError::Unreachable(m) | TokenError::Rejected(m) | TokenError::Unexpected(m) => m,
        }
    }

    /// Whether Google itself said the grant is gone. Only this justifies
    /// sending the user back to the connect screen.
    pub fn is_rejection(&self) -> bool {
        matches!(self, TokenError::Rejected(_))
    }
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for TokenError {}

pub fn refresh_access_token(refresh_token: &str) -> Result<AccessToken, TokenError> {
    let config = ClientConfig::load().map_err(TokenError::Unexpected)?;

    let response = post_token(token_form(
        &config,
        vec![
            ("refresh_token", refresh_token.to_string()),
            ("grant_type", "refresh_token".to_string()),
        ],
    ))?;

    Ok(AccessToken::new(response.access_token, response.expires_in))
}

fn post_token(form: Vec<(&str, String)>) -> Result<TokenResponse, TokenError> {
    let client = http_client().map_err(TokenError::Unexpected)?;

    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .map_err(|e| {
            TokenError::Unreachable(format!(
                "Could not reach Google to complete sign-in: {}",
                redact(e)
            ))
        })?;

    let status = response.status();
    let body = response.text().map_err(|e| {
        TokenError::Unexpected(format!("Google's reply could not be read: {}", redact(e)))
    })?;

    if status.is_success() {
        return serde_json::from_str(&body).map_err(|_| {
            TokenError::Unexpected("Google's reply was not in the expected format.".to_string())
        });
    }

    // Google's own trouble, not a verdict on the grant. Retrying is right.
    if status.is_server_error() {
        return Err(TokenError::Unexpected(format!(
            "Google is having trouble ({status}). Your tasks will sync when it recovers."
        )));
    }

    // The body can echo request parameters, so surface only the error code.
    let parsed: Option<ErrorResponse> = serde_json::from_str(&body).ok();
    let code = parsed
        .as_ref()
        .and_then(|e| e.error.as_deref())
        .unwrap_or("unknown_error");
    let description = parsed
        .as_ref()
        .and_then(|e| e.error_description.as_deref())
        .unwrap_or("");

    log::warn!("token endpoint returned {status}: {code}");

    Err(TokenError::Rejected(match code {
        "invalid_grant" => {
            "Your Google sign-in has expired. Connect again to continue.".into()
        }
        "invalid_client" => {
            "The Google client credentials were rejected. Check the client_secret \
             JSON file."
                .into()
        }
        _ if description.is_empty() => format!("Google rejected the sign-in ({code})."),
        _ => format!("Google rejected the sign-in ({code}): {description}"),
    }))
}

/// Best-effort. ARCHITECTURE §4.4: local credentials are deleted whether or not
/// this succeeds, so a network failure can never leave them behind.
pub fn revoke(refresh_token: &str) -> Result<(), String> {
    let client = http_client()?;
    client
        .post(REVOKE_ENDPOINT)
        .form(&[("token", refresh_token)])
        .send()
        .map_err(|e| format!("Could not reach Google to revoke access: {}", redact(e)))?;
    Ok(())
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Could not start a secure connection: {}", redact(e)))
}

/// reqwest error strings include the request URL, which for the token endpoint
/// is harmless but for anything else could carry parameters. Strip it.
fn redact(err: reqwest::Error) -> String {
    let without_url = err.without_url();
    without_url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ClientConfig {
        ClientConfig {
            client_id: "test-client-id.apps.googleusercontent.com".into(),
            client_secret: Some("test-secret".into()),
        }
    }

    #[test]
    fn auth_url_requests_exactly_one_scope() {
        let pkce = Pkce::generate().unwrap();
        let url = build_auth_url(&config(), &pkce, "http://127.0.0.1:1234");

        assert!(url.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Ftasks"));
        for forbidden in ["gmail", "drive", "calendar", "contacts", "userinfo"] {
            assert!(!url.contains(forbidden), "url must not request {forbidden}");
        }
    }

    #[test]
    fn auth_url_uses_pkce_and_asks_for_a_refresh_token() {
        let pkce = Pkce::generate().unwrap();
        let url = build_auth_url(&config(), &pkce, "http://127.0.0.1:1234");

        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("access_type=offline"));
        // The verifier must never appear in a URL that reaches the browser.
        assert!(!url.contains(&pkce.verifier));
    }

    #[test]
    fn auth_url_carries_the_state_for_later_verification() {
        let pkce = Pkce::generate().unwrap();
        let url = build_auth_url(&config(), &pkce, "http://127.0.0.1:1234");
        assert!(url.contains(&format!("state={}", pkce.state)));
    }

    #[test]
    fn token_form_carries_the_secret_when_one_exists() {
        let form = token_form(&config(), vec![("grant_type", "refresh_token".into())]);
        let keys: Vec<&str> = form.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"client_id"));
        assert!(
            keys.contains(&"client_secret"),
            "Google's Desktop clients require it — measured, not assumed"
        );
    }

    #[test]
    fn token_form_omits_the_secret_when_there_is_none() {
        let config = ClientConfig {
            client_id: "id".into(),
            client_secret: None,
        };
        let form = token_form(&config, vec![("grant_type", "refresh_token".into())]);
        let keys: Vec<&str> = form.iter().map(|(k, _)| *k).collect();
        assert!(!keys.contains(&"client_secret"));
    }

    #[test]
    fn client_config_display_never_reveals_the_secret() {
        let rendered = format!("{}", config());
        assert!(!rendered.contains("test-secret"));
        assert!(!rendered.contains("test-client-id"));
    }
}
