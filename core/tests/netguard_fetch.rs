use musicwasp_core::netguard::{self, Access};
use musicwasp_core::podcast;
use std::io::{Read, Write};
use std::net::TcpListener;

const FEED: &str = r#"<rss version="2.0"><channel><title>Local Show</title>
<item><title>Ep</title><guid>g1</guid><enclosure url="http://127.0.0.1/x.mp3" type="audio/mpeg"/></item>
</channel></rss>"#;

/// Serve one canned HTTP response on 127.0.0.1 and return the port.
fn serve_feed() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer);
            let body = FEED;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

#[test]
fn public_only_refuses_a_loopback_feed_by_address() {
    let port = serve_feed();
    let url = format!("http://127.0.0.1:{port}/feed.xml");
    let error = podcast::fetch(&url, Access::PublicOnly).unwrap_err();
    assert!(
        format!("{error:#}").contains("local or private address"),
        "{error:#}"
    );
}

#[test]
fn public_only_refuses_a_loopback_feed_by_name() {
    let port = serve_feed();
    let url = format!("http://localhost:{port}/feed.xml");
    let error = podcast::fetch(&url, Access::PublicOnly).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("local or private address"), "{text}");
}

#[test]
fn any_host_still_reaches_a_local_feed() {
    let port = serve_feed();
    let url = format!("http://127.0.0.1:{port}/feed.xml");
    let feed = podcast::fetch(&url, Access::AnyHost).unwrap();
    assert_eq!(feed.title, "Local Show");
    assert_eq!(feed.episodes.len(), 1);
}

#[test]
fn public_only_client_cannot_reach_a_loopback_name() {
    let port = serve_feed();
    let client = netguard::configure(reqwest::blocking::Client::builder(), Access::PublicOnly)
        .build()
        .unwrap();
    let result = client.get(format!("http://localhost:{port}/x")).send();
    assert!(result.is_err());
}

#[test]
fn any_host_client_reaches_a_loopback_name() {
    let port = serve_feed();
    let client = netguard::configure(reqwest::blocking::Client::builder(), Access::AnyHost)
        .build()
        .unwrap();
    let response = client
        .get(format!("http://localhost:{port}/x"))
        .send()
        .unwrap();
    assert!(response.status().is_success());
}

#[test]
fn invalid_feed_url_is_reported() {
    let error = podcast::fetch("not a url", Access::PublicOnly).unwrap_err();
    assert!(format!("{error:#}").contains("invalid feed URL"));
}
