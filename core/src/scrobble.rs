//! ListenBrainz scrobbling.
//!
//! ListenBrainz needs a user token, not a developer API key. The token is sent
//! in the `Authorization` header and is never logged by this module.

use anyhow::{Context, Result};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

const SUBMIT_URL: &str = "https://api.listenbrainz.org/1/submit-listens";
const USER_AGENT: &str = concat!("MusicWasp/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct Listen {
    pub artist: String,
    pub title: String,
    pub release: String,
    pub duration_secs: u64,
    /// Unix timestamp when playback of this track started.
    pub started_at: i64,
}

#[derive(Serialize)]
struct SubmitBody<'a> {
    listen_type: &'a str,
    payload: Vec<Payload<'a>>,
}

#[derive(Serialize)]
struct Payload<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    listened_at: Option<i64>,
    track_metadata: TrackMetadata<'a>,
}

#[derive(Serialize)]
struct TrackMetadata<'a> {
    artist_name: &'a str,
    track_name: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    release_name: &'a str,
    additional_info: AdditionalInfo,
}

#[derive(Serialize)]
struct AdditionalInfo {
    submission_client: &'static str,
    submission_client_version: &'static str,
    media_player: &'static str,
    duration_ms: u64,
}

/// Submit a single completed listen.
pub fn submit_single(token: &str, listen: &Listen) -> Result<()> {
    submit(token, "single", Some(listen.started_at), listen)
}

/// Tell ListenBrainz what is playing right now.
pub fn submit_now_playing(token: &str, listen: &Listen) -> Result<()> {
    submit(token, "playing_now", None, listen)
}

fn submit(token: &str, listen_type: &str, listened_at: Option<i64>, listen: &Listen) -> Result<()> {
    if token.trim().is_empty() {
        anyhow::bail!("ListenBrainz token is empty");
    }
    let body = SubmitBody {
        listen_type,
        payload: vec![Payload {
            listened_at,
            track_metadata: TrackMetadata {
                artist_name: &listen.artist,
                track_name: &listen.title,
                release_name: &listen.release,
                additional_info: AdditionalInfo {
                    submission_client: "MusicWasp",
                    submission_client_version: env!("CARGO_PKG_VERSION"),
                    media_player: "MusicWasp",
                    duration_ms: listen.duration_secs.saturating_mul(1000),
                },
            },
        }],
    };

    let response = listenbrainz_client()?
        .post(SUBMIT_URL)
        .header("Authorization", format!("Token {}", token.trim()))
        .json(&body)
        .send()
        .context("ListenBrainz request failed")?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().unwrap_or_default();
        anyhow::bail!("ListenBrainz returned {status}: {}", text.trim());
    }
    Ok(())
}

fn listenbrainz_client() -> Result<reqwest::blocking::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let built = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("cannot build ListenBrainz client")?;
    Ok(CLIENT.get_or_init(|| built).clone())
}

/// Unix timestamp for "now".
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tokens_are_rejected_before_any_request() {
        let listen = Listen {
            artist: "Artist".into(),
            title: "Title".into(),
            release: "Album".into(),
            duration_secs: 180,
            started_at: 0,
        };
        assert!(submit_single("", &listen).is_err());
        assert!(submit_now_playing("   ", &listen).is_err());
    }
}
