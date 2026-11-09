//! Endpoint overrides for development and tests.
//!
//! Redirecting a network API is honoured only in debug builds. A release binary
//! ignores these variables entirely, so an environment variable cannot send the
//! AcoustID key or audio fingerprints to another host.

/// The override from the environment, or `None` in release builds.
pub fn endpoint_override(name: &str) -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}
