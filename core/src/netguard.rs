//! Network policy for content the user did not type: podcast feeds, podcast
//! enclosures and the redirects they cause.
//!
//! A subscribed feed chooses the hosts the app contacts. Without this policy it
//! could name `localhost`, a router's admin page or a cloud metadata address.
//! With [`Access::PublicOnly`] the app connects only to public addresses.
//!
//! Two paths are checked. IP-literal hosts such as `http://127.0.0.1/` never reach
//! a DNS resolver, so [`check_url`] rejects them up front and the redirect policy
//! rechecks each hop. Host names go through a resolver that returns only public
//! addresses. reqwest connects to exactly those addresses, so a name that later
//! resolves to a private address cannot be reached.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::{self, Attempt};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;

/// Which destinations a request may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Public addresses only. Used for podcast feeds and enclosures by default.
    PublicOnly,
    /// Any address, including local ones. Used for radio streams the user typed,
    /// and for podcasts once the user enables local feeds.
    AnyHost,
}

/// Redirects followed before a request gives up.
const MAX_REDIRECTS: usize = 10;

/// True when `ip` is a public, routable address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_multicast()
        || a == 0
        || (a == 100 && (b & 0xc0) == 64) // 100.64.0.0/10, carrier-grade NAT
        || (a == 192 && b == 0 && c == 0) // 192.0.0.0/24, IETF protocol assignments
        || a >= 240) // 240.0.0.0/4, reserved
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    let s = address.segments();
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) embed an IPv4 address that
    // would reach the local network, so judge the embedded address.
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        return is_public_v4(embedded_v4(&s));
    }
    if s[0] == 0x2002 {
        return is_public_v4(Ipv4Addr::new(
            (s[1] >> 8) as u8,
            s[1] as u8,
            (s[2] >> 8) as u8,
            s[2] as u8,
        ));
    }
    // Deprecated IPv4-compatible form (::a.b.c.d). Rarely routed, but it
    // embeds an address, so judge that address.
    if s[..6].iter().all(|&segment| segment == 0) {
        return is_public_v4(embedded_v4(&s));
    }
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00 // fc00::/7, unique local
        || (s[0] & 0xffc0) == 0xfe80 // fe80::/10, link-local
        || (s[0] & 0xffc0) == 0xfec0) // fec0::/10, site-local (deprecated)
}

fn embedded_v4(s: &[u16; 8]) -> Ipv4Addr {
    Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8)
}

/// Reject an IP-literal host the policy forbids. Host names are not resolved
/// here; the resolver checks them when the request connects.
pub fn check_url(url: &reqwest::Url, access: Access) -> Result<(), String> {
    if access == Access::AnyHost {
        return Ok(());
    }
    let host = url.host_str().unwrap_or_default();
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    match literal.parse::<IpAddr>() {
        Ok(ip) if !is_public(ip) => Err(refusal(literal)),
        _ => Ok(()),
    }
}

fn refusal(host: &str) -> String {
    format!(
        "{host} is a local or private address; enable local feeds in Settings → Track options to use it"
    )
}

/// Keep only the addresses the policy allows.
pub fn allowed_addrs<I>(addrs: I, access: Access) -> Vec<SocketAddr>
where
    I: IntoIterator<Item = SocketAddr>,
{
    addrs
        .into_iter()
        .filter(|addr| access == Access::AnyHost || is_public(addr.ip()))
        .collect()
}

/// Resolver that returns public addresses only.
struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            // Blocking lookup on the client's worker. Names are short-lived and
            // this runs once per connection.
            let found: Vec<SocketAddr> = (host.as_str(), 0)
                .to_socket_addrs()
                .map_err(|error| format!("cannot resolve {host}: {error}"))?
                .collect();
            let allowed = allowed_addrs(found, Access::PublicOnly);
            if allowed.is_empty() {
                return Err(refusal(&host).into());
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

/// Apply the policy to a client builder: the resolver for names, and a redirect
/// policy that rechecks every hop. Timeouts and the user agent stay with the caller.
pub fn configure(
    builder: reqwest::blocking::ClientBuilder,
    access: Access,
) -> reqwest::blocking::ClientBuilder {
    let builder = builder.redirect(redirect_policy(access));
    match access {
        Access::AnyHost => builder,
        Access::PublicOnly => builder.dns_resolver(Arc::new(PublicResolver)),
    }
}

/// Decide whether one redirect hop may be followed. Split out from the reqwest
/// policy so the decision itself can be tested.
fn check_redirect(previous: usize, url: &reqwest::Url, access: Access) -> Result<(), String> {
    if previous >= MAX_REDIRECTS {
        return Err("too many redirects".to_string());
    }
    check_url(url, access)
}

fn redirect_policy(access: Access) -> redirect::Policy {
    redirect::Policy::custom(move |attempt: Attempt| {
        match check_redirect(attempt.previous().len(), attempt.url(), access) {
            Ok(()) => attempt.follow(),
            Err(message) => attempt.error(message),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn refuses_local_and_private_ipv4() {
        for ip in [
            v4(127, 0, 0, 1),
            v4(10, 1, 2, 3),
            v4(172, 16, 0, 1),
            v4(172, 31, 255, 254),
            v4(192, 168, 1, 1),
            v4(169, 254, 169, 254),
            v4(100, 64, 0, 1),
            v4(0, 0, 0, 0),
            v4(255, 255, 255, 255),
            v4(224, 0, 0, 1),
            v4(240, 0, 0, 1),
        ] {
            assert!(!is_public(ip), "{ip} should be refused");
        }
    }

    #[test]
    fn allows_public_ipv4() {
        for ip in [
            v4(8, 8, 8, 8),
            v4(1, 1, 1, 1),
            v4(172, 15, 0, 1),
            v4(172, 32, 0, 1),
            v4(100, 128, 0, 1),
            v4(192, 169, 0, 1),
        ] {
            assert!(is_public(ip), "{ip} should be allowed");
        }
    }

    #[test]
    fn refuses_local_and_private_ipv6() {
        for text in [
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
        ] {
            let ip: IpAddr = text.parse().unwrap();
            assert!(!is_public(ip), "{text} should be refused");
        }
    }

    #[test]
    fn allows_public_ipv6() {
        let ip: IpAddr = "2606:4700:4700::1111".parse().unwrap();
        assert!(is_public(ip));
    }

    #[test]
    fn judges_ipv4_mapped_and_embedded_addresses() {
        for text in [
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.10",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "2002:c0a8:0001::1",
        ] {
            let ip: IpAddr = text.parse().unwrap();
            assert!(!is_public(ip), "{text} should be refused");
        }
        let public: IpAddr = "::ffff:8.8.8.8".parse().unwrap();
        assert!(is_public(public));
    }

    #[test]
    fn literal_check_refuses_only_forbidden_hosts() {
        let local = reqwest::Url::parse("http://127.0.0.1:8000/feed.xml").unwrap();
        assert!(check_url(&local, Access::PublicOnly).is_err());
        assert!(check_url(&local, Access::AnyHost).is_ok());

        let v6 = reqwest::Url::parse("http://[::1]/feed.xml").unwrap();
        assert!(check_url(&v6, Access::PublicOnly).is_err());

        let public = reqwest::Url::parse("https://8.8.8.8/feed.xml").unwrap();
        assert!(check_url(&public, Access::PublicOnly).is_ok());

        // The URL parser normalizes alternate IPv4 spellings before we see them.
        for text in [
            "http://127.1/feed.xml",
            "http://2130706433/feed.xml",
            "http://0x7f000001/feed.xml",
            "http://0177.0.0.1/feed.xml",
        ] {
            let url = reqwest::Url::parse(text).unwrap();
            assert!(
                check_url(&url, Access::PublicOnly).is_err(),
                "{text} parsed as {} and was allowed",
                url.host_str().unwrap_or_default()
            );
        }

        // Names are left to the resolver.
        let name = reqwest::Url::parse("https://feeds.example.com/a.xml").unwrap();
        assert!(check_url(&name, Access::PublicOnly).is_ok());
    }

    #[test]
    fn redirects_to_local_addresses_are_refused() {
        let local = reqwest::Url::parse("http://192.168.1.10/admin").unwrap();
        assert!(check_redirect(0, &local, Access::PublicOnly).is_err());
        assert!(check_redirect(0, &local, Access::AnyHost).is_ok());

        let public = reqwest::Url::parse("https://example.com/episode.mp3").unwrap();
        assert!(check_redirect(0, &public, Access::PublicOnly).is_ok());

        // A chain that has already used every allowed hop stops here.
        assert!(check_redirect(MAX_REDIRECTS, &public, Access::AnyHost).is_err());
    }

    #[test]
    fn filter_keeps_only_public_addresses() {
        let found = vec![
            "127.0.0.1:80".parse().unwrap(),
            "8.8.8.8:80".parse().unwrap(),
            "10.0.0.5:80".parse().unwrap(),
        ];
        let kept = allowed_addrs(found.clone(), Access::PublicOnly);
        assert_eq!(kept, vec!["8.8.8.8:80".parse::<SocketAddr>().unwrap()]);
        assert_eq!(allowed_addrs(found, Access::AnyHost).len(), 3);
    }
}
