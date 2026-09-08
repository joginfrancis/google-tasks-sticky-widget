//! API error classification.
//!
//! The point of this module is that callers branch on *meaning*, not on status
//! codes — a 403 that means "rate limited" and a 403 that means "you lost
//! access" need opposite responses (ARCHITECTURE §5.4).

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// Credentials are gone or rejected. Refresh once, then disconnect.
    Unauthorized,
    /// Rate limited or quota exhausted. Back off; optional server-suggested wait.
    RateLimited { retry_after_secs: Option<u64> },
    /// Permission genuinely revoked — not a rate limit wearing a 403.
    Forbidden,
    /// Task or list is gone. Drop it from the cache; not an error to surface.
    NotFound,
    /// Google's side. Retry with backoff.
    ServerError(u16),
    /// No network, DNS failure, timeout. Offline, not broken.
    Network,
    /// Response could not be parsed. Not retryable.
    Malformed(String),
}

impl ApiError {
    /// Whether a retry could plausibly succeed without user action.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ApiError::RateLimited { .. } | ApiError::ServerError(_) | ApiError::Network
        )
    }

    /// SPEC §5: written for a person, not a protocol.
    pub fn user_message(&self) -> String {
        match self {
            ApiError::Unauthorized => {
                "Google access expired. Reconnect in Settings.".into()
            }
            ApiError::Forbidden => {
                "Google denied access to your tasks. Try reconnecting in Settings."
                    .into()
            }
            ApiError::RateLimited { .. } => {
                "Google is asking us to slow down. Retrying shortly.".into()
            }
            ApiError::NotFound => "That task no longer exists.".into(),
            ApiError::ServerError(_) => {
                "Google Tasks is having trouble. Retrying shortly.".into()
            }
            ApiError::Network => {
                "Can't reach Google. Showing your last synced tasks.".into()
            }
            ApiError::Malformed(_) => {
                "Google sent something unexpected. Try syncing again.".into()
            }
        }
    }

    /// Google returns 403 for both quota problems and genuine permission loss.
    /// The status alone cannot tell them apart, so the error body's `reason`
    /// decides — treating a quota 403 as revoked access would sign the user out
    /// over a burst of requests.
    pub fn from_status(status: u16, body: &str, retry_after: Option<u64>) -> Self {
        match status {
            401 => ApiError::Unauthorized,
            403 => {
                let quota_related = ["rateLimitExceeded", "userRateLimitExceeded",
                                     "quotaExceeded", "dailyLimitExceeded"]
                    .iter()
                    .any(|reason| body.contains(reason));

                if quota_related {
                    ApiError::RateLimited {
                        retry_after_secs: retry_after,
                    }
                } else {
                    ApiError::Forbidden
                }
            }
            404 => ApiError::NotFound,
            429 => ApiError::RateLimited {
                retry_after_secs: retry_after,
            },
            500..=599 => ApiError::ServerError(status),
            other => ApiError::Malformed(format!("unexpected status {other}")),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_403_is_a_rate_limit_not_a_revocation() {
        let body = r#"{"error":{"errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        assert_eq!(
            ApiError::from_status(403, body, None),
            ApiError::RateLimited {
                retry_after_secs: None
            }
        );
    }

    #[test]
    fn permission_403_is_forbidden() {
        let body = r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#;
        assert_eq!(ApiError::from_status(403, body, None), ApiError::Forbidden);
    }

    #[test]
    fn retryability_matches_what_a_retry_could_fix() {
        assert!(ApiError::Network.is_retryable());
        assert!(ApiError::ServerError(503).is_retryable());
        assert!(ApiError::RateLimited {
            retry_after_secs: Some(30)
        }
        .is_retryable());

        // Retrying these just burns quota.
        assert!(!ApiError::Unauthorized.is_retryable());
        assert!(!ApiError::Forbidden.is_retryable());
        assert!(!ApiError::NotFound.is_retryable());
        assert!(!ApiError::Malformed("bad".into()).is_retryable());
    }

    #[test]
    fn messages_never_leak_protocol_detail() {
        for error in [
            ApiError::Unauthorized,
            ApiError::Forbidden,
            ApiError::NotFound,
            ApiError::ServerError(500),
            ApiError::Network,
            ApiError::Malformed("serde error at line 4".into()),
        ] {
            let message = error.user_message();
            for jargon in ["401", "403", "404", "500", "serde", "exception", "null"] {
                assert!(
                    !message.contains(jargon),
                    "{message:?} leaks {jargon}"
                );
            }
        }
    }
}
