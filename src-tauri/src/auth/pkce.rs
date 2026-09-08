//! PKCE verifier/challenge and CSRF state generation.
//!
//! ARCHITECTURE §4.2: PKCE is what actually secures this flow. The redirect is
//! a loopback address, so any other process on the machine could in principle
//! race for the authorization code — without the verifier, holding that code
//! would be enough to mint tokens.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use constant_time_eq::constant_time_eq;
use sha2::{Digest, Sha256};

/// 32 bytes of entropy. The spec permits 43–128 characters of verifier; 32
/// random bytes base64url-encodes to 43, the shortest length that is still the
/// full strength the spec intends.
const ENTROPY_BYTES: usize = 32;

/// Straight from the OS CSPRNG. Not a seeded generator — this is key material,
/// and a fallible call that surfaces the failure is the right shape for it.
fn random_urlsafe(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf)
        .map_err(|e| format!("secure random number generation failed: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    pub fn generate() -> Result<Self, String> {
        let verifier = random_urlsafe(ENTROPY_BYTES)?;
        let state = random_urlsafe(ENTROPY_BYTES)?;

        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(digest);

        Ok(Self {
            verifier,
            challenge,
            state,
        })
    }

    /// Constant-time so a mismatch cannot be narrowed down by timing. The
    /// length check leaks only length, which is fixed and public.
    pub fn state_matches(&self, candidate: &str) -> bool {
        let expected = self.state.as_bytes();
        let actual = candidate.as_bytes();
        expected.len() == actual.len() && constant_time_eq(expected, actual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_meets_rfc7636_length_bounds() {
        let pkce = Pkce::generate().unwrap();
        assert!(pkce.verifier.len() >= 43, "verifier too short");
        assert!(pkce.verifier.len() <= 128, "verifier too long");
    }

    #[test]
    fn challenge_is_sha256_of_verifier_base64url_unpadded() {
        let pkce = Pkce::generate().unwrap();
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        assert!(!pkce.challenge.contains('='), "challenge must be unpadded");
        assert!(!pkce.challenge.contains('+') && !pkce.challenge.contains('/'));
    }

    #[test]
    fn each_generation_is_unique() {
        let a = Pkce::generate().unwrap();
        let b = Pkce::generate().unwrap();
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state);
    }

    #[test]
    fn state_matches_only_the_exact_value() {
        let pkce = Pkce::generate().unwrap();
        assert!(pkce.state_matches(&pkce.state));
        assert!(!pkce.state_matches(""));
        assert!(!pkce.state_matches("wrong"));

        // A prefix must not pass — the classic bug this guards against.
        let prefix = &pkce.state[..pkce.state.len() - 1];
        assert!(!pkce.state_matches(prefix));

        let mut altered = pkce.state.clone();
        altered.pop();
        altered.push(if pkce.state.ends_with('A') { 'B' } else { 'A' });
        assert!(!pkce.state_matches(&altered));
    }
}
