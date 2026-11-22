//! Bounded download for remote artwork used by podcasts and radio stations.
//!
//! The GUI decodes the returned bytes. This module only enforces the network
//! policy and a hard byte limit so a hostile image URL cannot exhaust memory.

use crate::netguard::{self, Access};
use anyhow::{Context, Result};
use std::time::Duration;

const MAX_ARTWORK_BYTES: u64 = 8 * 1024 * 1024;

const USER_AGENT: &str = concat!("MusicWasp/", env!("CARGO_PKG_VERSION"));

/// Download one artwork URL after applying the same address policy as feeds
/// and streams. `access` is [`Access::PublicOnly`] for podcast artwork and
/// [`Access::AnyHost`] for user-entered radio covers.
pub fn fetch(url: &str, access: Access) -> Result<Vec<u8>> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("invalid artwork URL {url}"))?;
    netguard::check_url(&parsed, access).map_err(anyhow::Error::msg)?;
    let client = netguard::configure(
        reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25)),
        access,
    )
    .build()
    .context("cannot build HTTP client")?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("cannot fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("{url} returned an error"))?;
    crate::bounded::read_capped(response, MAX_ARTWORK_BYTES)
        .with_context(|| format!("cannot read {url}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn rejects_invalid_urls_without_a_request() {
        assert!(fetch("not a url", Access::PublicOnly).is_err());
    }

    #[test]
    fn public_only_refuses_loopback_before_connecting() {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(!crate::netguard::is_public(loopback));
        assert!(fetch("http://127.0.0.1/cover.png", Access::PublicOnly).is_err());
    }
}
