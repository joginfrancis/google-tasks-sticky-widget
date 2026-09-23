//! Loading the Google OAuth desktop client credentials.
//!
//! ARCHITECTURE §4.2: a desktop client secret is NOT a confidential credential —
//! Google documents it as such, and PKCE is what secures the flow. It lives
//! outside the repo purely so it never lands in git, not because hiding it would
//! achieve anything.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

/// Overrides the default location. Set at build or run time to point at a
/// different client, e.g. for a second Google account.
const ENV_OVERRIDE: &str = "GTASKS_CLIENT_SECRET_FILE";

/// Where `client_secret_*.json` is expected when the env var is unset.
const DEFAULT_DIR: &str = ".gtasks-widget";

#[derive(Debug, Deserialize)]
struct GoogleCredentialsFile {
    installed: Option<InstalledClient>,
    web: Option<InstalledClient>,
}

#[derive(Debug, Deserialize)]
struct InstalledClient {
    client_id: String,
    client_secret: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    auth_uri: String,
    #[allow(dead_code)]
    #[serde(default)]
    token_uri: String,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
}

fn candidate_paths() -> Vec<PathBuf> {
    if let Ok(explicit) = std::env::var(ENV_OVERRIDE) {
        return vec![PathBuf::from(explicit)];
    }

    let Some(home) = dirs_home() else {
        return Vec::new();
    };

    let dir = home.join(DEFAULT_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "json")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("client_secret"))
        })
        .collect();

    // Deterministic pick when several are present, rather than filesystem order.
    found.sort();
    found
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Compiled in by build.rs from whatever credentials the build machine had.
/// `None` in a build that had none — a development case, not a user one.
fn builtin() -> Option<ClientConfig> {
    let id = option_env!("GTASKS_BUILTIN_CLIENT_ID")?;
    Some(ClientConfig {
        client_id: id.to_string(),
        client_secret: option_env!("GTASKS_BUILTIN_CLIENT_SECRET").map(str::to_string),
    })
}

impl ClientConfig {
    pub fn load() -> Result<Self, String> {
        let paths = candidate_paths();

        let Some(path) = paths.first() else {
            // Nothing on this machine, so use what the app was built with —
            // the ordinary case for anyone who has simply installed it. A file
            // still wins where one exists, which is how a second account is
            // tested without rebuilding.
            if let Some(config) = builtin() {
                return Ok(config);
            }
            return Err(format!(
                "This copy of the app was built without Google credentials. Put a \
                 client_secret_*.json in %USERPROFILE%\\{DEFAULT_DIR}\\, or set \
                 {ENV_OVERRIDE} to its full path."
            ));
        };

        let raw = fs::read_to_string(path)
            .map_err(|e| format!("Could not read Google client credentials: {e}"))?;

        let parsed: GoogleCredentialsFile = serde_json::from_str(&raw).map_err(|e| {
            format!("Google client credentials file is not valid JSON: {e}")
        })?;

        let client = parsed
            .installed
            .or(parsed.web)
            .ok_or_else(|| {
                "Credentials file has no \"installed\" section — is it a Desktop \
                 app OAuth client?"
                    .to_string()
            })?;

        Ok(Self {
            client_id: client.client_id,
            client_secret: client.client_secret,
        })
    }
}

// Deliberately no Display/Debug that prints the secret: ARCHITECTURE §7.3.
impl std::fmt::Display for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ClientConfig {{ client_id: <redacted>, client_secret: {} }}",
            if self.client_secret.is_some() {
                "<present>"
            } else {
                "<absent>"
            }
        )
    }
}
