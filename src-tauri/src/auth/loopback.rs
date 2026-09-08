//! One-shot loopback HTTP listener for the OAuth redirect.
//!
//! Google removed the out-of-band copy/paste flow and discourages custom URI
//! schemes for desktop apps, so a loopback redirect is the supported route
//! (docs/api-findings.md §8). Deliberately hand-rolled: this needs to accept
//! exactly one request and then stop existing, which is less code than wiring in
//! an HTTP server, and less surface too.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

/// The browser tab is a one-shot; if the user wanders off, stop waiting.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Refuse absurd request lines rather than reading unbounded input from a
/// socket anything on the machine can connect to.
const MAX_REQUEST_LINE: u64 = 8 * 1024;

pub struct Loopback {
    listener: TcpListener,
    pub redirect_uri: String,
}

#[derive(Debug)]
pub enum CallbackResult {
    Success { code: String, state: String },
    /// Google reported an error, e.g. the user pressed Cancel.
    Denied { error: String, state: Option<String> },
}

impl Loopback {
    /// Binds to an OS-assigned port on 127.0.0.1 — never 0.0.0.0, which would
    /// expose the callback to the local network.
    pub fn bind() -> Result<Self, String> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .map_err(|e| format!("Could not open the local sign-in listener: {e}"))?;

        let port = listener
            .local_addr()
            .map_err(|e| format!("Could not read local listener address: {e}"))?
            .port();

        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{port}"),
        })
    }

    /// Blocks until the browser hits the redirect, or the timeout expires.
    ///
    /// Anything on the machine can connect to this port, so a connection that
    /// isn't the OAuth redirect is ignored and the listener keeps waiting rather
    /// than aborting the sign-in.
    pub fn wait_for_callback(self) -> Result<CallbackResult, String> {
        self.listener
            .set_nonblocking(false)
            .map_err(|e| format!("Listener configuration failed: {e}"))?;

        let deadline = std::time::Instant::now() + TIMEOUT;

        for stream in self.listener.incoming() {
            if std::time::Instant::now() > deadline {
                return Err("Sign-in timed out. Try connecting again.".into());
            }

            let mut stream = match stream {
                Ok(s) => s,
                Err(err) => {
                    log::debug!("ignoring failed loopback connection: {err}");
                    continue;
                }
            };

            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

            match read_query(&mut stream) {
                Ok(Some(params)) => {
                    let result = interpret(params);
                    respond(&mut stream, &result);
                    return Ok(result);
                }
                Ok(None) => {
                    // Not the redirect — a browser probe, a port scanner, a
                    // favicon request. Answer briefly and keep listening.
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
                Err(err) => log::debug!("unreadable loopback request: {err}"),
            }
        }

        Err("The sign-in listener closed unexpectedly.".into())
    }
}

/// Returns Ok(None) when the request is not the OAuth redirect.
fn read_query(stream: &mut TcpStream) -> Result<Option<HashMap<String, String>>, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();

    reader
        .by_ref()
        .take(MAX_REQUEST_LINE)
        .read_line(&mut request_line)
        .map_err(|e| format!("read failed: {e}"))?;

    // "GET /?code=...&state=... HTTP/1.1"
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method != "GET" {
        return Ok(None);
    }

    let Some((_, query)) = target.split_once('?') else {
        return Ok(None);
    };

    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter_map(|(k, v)| {
            let key = urlencoding::decode(k).ok()?.into_owned();
            let value = urlencoding::decode(v).ok()?.into_owned();
            Some((key, value))
        })
        .collect();

    if params.contains_key("code") || params.contains_key("error") {
        Ok(Some(params))
    } else {
        Ok(None)
    }
}

fn interpret(mut params: HashMap<String, String>) -> CallbackResult {
    let state = params.remove("state");

    if let Some(error) = params.remove("error") {
        return CallbackResult::Denied { error, state };
    }

    match (params.remove("code"), state) {
        (Some(code), Some(state)) => CallbackResult::Success { code, state },
        // A code without state is unverifiable, so treat it as a failure rather
        // than trusting it.
        (Some(_), None) => CallbackResult::Denied {
            error: "missing_state".into(),
            state: None,
        },
        _ => CallbackResult::Denied {
            error: "invalid_response".into(),
            state: None,
        },
    }
}

fn respond(stream: &mut TcpStream, result: &CallbackResult) {
    let (heading, detail) = match result {
        CallbackResult::Success { .. } => (
            "You're connected",
            "Sticky Widget now has access to your Google Tasks. You can close this tab.",
        ),
        CallbackResult::Denied { .. } => (
            "Sign-in cancelled",
            "Nothing was changed. You can close this tab and try again from the widget.",
        ),
    };

    // Self-contained: no external requests, nothing to leak a referrer to.
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <title>Sticky Widget</title><meta name=\"referrer\" content=\"no-referrer\">\
         <style>body{{font-family:'Segoe UI',system-ui,sans-serif;background:#fbfbfd;\
         color:#1a1a1f;display:grid;place-items:center;height:100vh;margin:0}}\
         .card{{text-align:center;max-width:22rem;padding:2rem}}\
         h1{{font-size:1.15rem;margin:0 0 .5rem}}p{{color:#5c5c66;line-height:1.5;margin:0}}\
         @media(prefers-color-scheme:dark){{body{{background:#202024;color:#ededf0}}\
         p{{color:#b4b4be}}}}</style></head>\
         <body><div class=\"card\"><h1>{heading}</h1><p>{detail}</p></div></body></html>"
    );

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );

    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_to_loopback_only() {
        let lb = Loopback::bind().unwrap();
        assert!(
            lb.redirect_uri.starts_with("http://127.0.0.1:"),
            "redirect must be loopback, got {}",
            lb.redirect_uri
        );
        let port: u16 = lb.redirect_uri.rsplit(':').next().unwrap().parse().unwrap();
        assert_ne!(port, 0, "OS should have assigned a real port");
    }

    #[test]
    fn success_requires_both_code_and_state() {
        let mut params = HashMap::new();
        params.insert("code".into(), "abc".into());
        assert!(matches!(
            interpret(params),
            CallbackResult::Denied { .. }
        ));

        let mut params = HashMap::new();
        params.insert("code".into(), "abc".into());
        params.insert("state".into(), "xyz".into());
        assert!(matches!(
            interpret(params),
            CallbackResult::Success { .. }
        ));
    }

    #[test]
    fn user_cancellation_is_reported_as_denied() {
        let mut params = HashMap::new();
        params.insert("error".into(), "access_denied".into());
        params.insert("state".into(), "xyz".into());
        match interpret(params) {
            CallbackResult::Denied { error, .. } => assert_eq!(error, "access_denied"),
            other => panic!("expected denial, got {other:?}"),
        }
    }
}
