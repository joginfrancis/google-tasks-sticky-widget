use std::path::PathBuf;

/// Bakes the Google OAuth *client* credentials into the binary.
///
/// A desktop client secret is not a confidential credential — Google documents
/// it as such, and PKCE is what secures the flow (docs/api-findings.md §8). It
/// is compiled in so that installing the app is all a new user has to do: the
/// alternative is asking every person to download a JSON file from a Cloud
/// project they do not own, which no one outside this repo can do.
///
/// The file itself stays out of git. A build without it still compiles, and
/// the app then falls back to the per-machine file, which is what development
/// on a second account uses.
fn main() {
    println!("cargo:rerun-if-env-changed=GTASKS_CLIENT_SECRET_FILE");

    if let Some((id, secret)) = read_credentials() {
        println!("cargo:rustc-env=GTASKS_BUILTIN_CLIENT_ID={id}");
        if let Some(secret) = secret {
            println!("cargo:rustc-env=GTASKS_BUILTIN_CLIENT_SECRET={secret}");
        }
    } else {
        println!(
            "cargo:warning=no Google client credentials baked in; the app will              look for client_secret_*.json at run time"
        );
    }

    tauri_build::build()
}

fn read_credentials() -> Option<(String, Option<String>)> {
    let path = credentials_path()?;
    println!("cargo:rerun-if-changed={}", path.display());

    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let client = parsed.get("installed").or_else(|| parsed.get("web"))?;

    let id = client.get("client_id")?.as_str()?.to_string();
    let secret = client
        .get("client_secret")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some((id, secret))
}

fn credentials_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("GTASKS_CLIENT_SECRET_FILE") {
        return Some(PathBuf::from(explicit));
    }

    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let dir = PathBuf::from(home).join(".gtasks-widget");
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "json")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("client_secret"))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}
