//! Headless UI regression tests.
//!
//! These drive the real `App` through egui_kittest's AccessKit tree. The
//! auto-tag flow talks to an in-process fake MusicBrainz server, so it never
//! touches the network.

use crate::app::{App, AutoTagStage};
use crate::i18n::Language;
use crate::settings::{TrackColumn, TrackLayout};
use crate::theme::{self, Skin};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use musicwasp_core::{tags, Library, TagEdit};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Set test-specific environment variables and restore them on drop. The lock
/// keeps parallel UI tests from fighting over process-wide env vars.
struct TestEnv {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl TestEnv {
    fn new(data_dir: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let names = [
            "MUSICWASP_TEST",
            "MUSICWASP_DATA_DIR",
            "MUSICWASP_MUSICBRAINZ_API",
            "MUSICWASP_COVERART_API",
            "MUSICWASP_SCREENSHOT",
            "MUSICWASP_SCREENSHOT_FRAME",
            "MUSICWASP_PLAIN",
            "MUSICWASP_AUTOPLAY",
            "MUSICWASP_SCAN",
        ];
        let saved = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        let mut env = Self { _lock: lock, saved };
        env.set("MUSICWASP_TEST", Some("1"));
        env.set(
            "MUSICWASP_DATA_DIR",
            Some(data_dir.to_string_lossy().as_ref()),
        );
        for name in [
            "MUSICWASP_MUSICBRAINZ_API",
            "MUSICWASP_COVERART_API",
            "MUSICWASP_SCREENSHOT",
            "MUSICWASP_SCREENSHOT_FRAME",
            "MUSICWASP_PLAIN",
            "MUSICWASP_AUTOPLAY",
            "MUSICWASP_SCAN",
        ] {
            env.set(name, None);
        }
        env
    }

    fn set(&mut self, name: &'static str, value: Option<&str>) {
        // Update the saved value only once, so Drop restores the original.
        if !self.saved.iter().any(|(saved, _)| *saved == name) {
            self.saved.push((name, std::env::var_os(name)));
        }
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn write_wav(path: &Path, frames: u32) {
    let sample_rate = 44_100u32;
    let data_len = frames;
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&8u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_len.to_le_bytes()).unwrap();
    file.write_all(&vec![0u8; data_len as usize]).unwrap();
}

struct SeedTrack {
    file: &'static str,
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    year: i32,
    track_no: i32,
}

fn seed_library(root: &Path, tracks: &[SeedTrack]) {
    let music = root.join("music");
    std::fs::create_dir_all(&music).unwrap();
    for track in tracks {
        let path = music.join(track.file);
        write_wav(&path, 800);
        tags::write(
            &path,
            &TagEdit {
                title: track.title.to_string(),
                artist: track.artist.to_string(),
                album: track.album.to_string(),
                year: Some(track.year),
                track_no: Some(track.track_no),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let lib = Library::open_at(&root.join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
}

fn harness() -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(|cc| App::new(cc))
}

fn text_inputs<'a>(harness: &'a Harness<'static, App>) -> Vec<egui_kittest::Node<'a>> {
    harness.get_all_by_role(Role::TextInput).collect::<Vec<_>>()
}

fn wait_until(
    harness: &mut Harness<'static, App>,
    timeout: Duration,
    mut done: impl FnMut(&App) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        harness.run_steps(2);
        if done(harness.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

// ---------------------------------------------------------------- fake server

struct FakeMusicBrainz {
    api: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeMusicBrainz {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);

        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Some(path) = serve(stream) {
                            thread_requests.lock().unwrap().push(path);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            api: format!("http://127.0.0.1:{port}/ws/2"),
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn caa_api(&self) -> String {
        self.api.replace("/ws/2", "/caa")
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeMusicBrainz {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut stream: TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8_lossy(&request);
    let path = text.lines().next()?.split_whitespace().nth(1)?.to_string();
    let route = path.split('?').next().unwrap_or(&path);

    let (status, content_type, body) = if route == "/ws/2/release" {
        (
            "200 OK",
            "application/json",
            r#"{"releases":[{"id":"fake-release","score":100,"title":"Fake Album","date":"1997-05-21","country":"GB","track-count":2,"artist-credit":[{"name":"Radiohead"}]}]}"#,
        )
    } else if route == "/ws/2/release/fake-release" {
        (
            "200 OK",
            "application/json",
            r#"{"id":"fake-release","title":"Fake Album","date":"1997-05-21","artist-credit":[{"name":"Radiohead"}],"release-group":{"first-release-date":"1997","tags":[{"name":"alternative rock","count":5}]},"media":[{"position":1,"tracks":[{"number":"1","title":"Airbag","length":284000,"artist-credit":[{"name":"Radiohead"}]},{"number":"2","title":"Paranoid Android","length":383000,"artist-credit":[{"name":"Radiohead"}]}]}],"cover-art-archive":{"front":false}}"#,
        )
    } else {
        ("404 Not Found", "text/plain", "not found")
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    Some(path)
}

// ---------------------------------------------------------------- tests

#[test]
fn custom_skin_json_adds_a_theme_and_icon() {
    let dir = tempfile::tempdir().unwrap();
    let _env = TestEnv::new(dir.path());
    let icon = dir.path().join("berry.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([220, 20, 90, 255]))
        .save(&icon)
        .unwrap();
    std::fs::write(
        dir.path().join("skins.json"),
        r##"{
            "skins": [
                {
                    "id": "berry",
                    "label": "Berry",
                    "aliases": ["berry-pink"],
                    "icon": "berry.png",
                    "palette": {
                        "accent": "#ff0066",
                        "bg_dark": "#1a0710",
                        "text": "#fff4f8"
                    }
                }
            ]
        }"##,
    )
    .unwrap();
    std::fs::write(dir.path().join("settings.json"), r#"{"skin":"berry"}"#).unwrap();

    let mut harness = harness();
    harness.run_steps(2);

    assert_eq!(harness.state().settings.skin.id(), "berry");
    assert_eq!(harness.state().settings.skin.label(), "Berry");
    let berry = Skin::from_id("berry-pink");
    assert_eq!(berry.id(), "berry");
    assert_eq!(berry.icon_path().as_deref(), Some(icon.as_path()));
    assert!(crate::icons::icon_data(berry).is_some());
    assert!(Skin::all().iter().any(|skin| skin.id() == "berry"));

    // Leave the built-in registry behind for the other tests.
    std::fs::remove_file(dir.path().join("skins.json")).unwrap();
    theme::load_user_skins();
}

#[test]
fn folder_context_menu_creates_a_playlist_in_that_folder() {
    let dir = tempfile::tempdir().unwrap();
    let _env = TestEnv::new(dir.path());
    let folder = {
        let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
        lib.create_playlist_folder("Setlists", None).unwrap()
    };

    let mut harness = harness();
    harness.run_steps(3);
    harness.get_by_label_contains("Setlists").click_secondary();
    harness.run_steps(1);
    harness.get_by_label("New playlist here…").click();
    harness.run_steps(2);
    assert!(harness.state().playlist_dialog_folder == Some(folder));
    assert!(harness.query_by_label_contains("Destination:").is_some());

    harness.get_by_label("Create").click();
    harness.run_steps(2);

    let playlists = &harness.state().playlists;
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].folder_id, Some(folder));
    assert_eq!(playlists[0].count, 0);
}

#[test]
fn can_create_an_empty_playlist_from_the_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let _env = TestEnv::new(dir.path());
    seed_library(
        dir.path(),
        &[SeedTrack {
            file: "one.wav",
            title: "One",
            artist: "Artist",
            album: "Album",
            year: 2024,
            track_no: 1,
        }],
    );

    let mut harness = harness();
    harness.run_steps(3);
    harness.state_mut().open_playlist_dialog(false, None);
    harness.run_steps(2);

    harness
        .get_by_role_and_label(Role::CheckBox, "Create empty playlist")
        .click_accesskit();
    harness.run_steps(1);
    assert!(harness.state().playlist_dialog_empty);

    harness.get_by_label("Create").click();
    harness.run_steps(2);

    let playlists = &harness.state().playlists;
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].count, 0, "the new playlist must be empty");
    assert!(harness.state().playlist_dialog.is_none());
}

#[test]
fn advanced_search_and_column_manager_update_the_ui() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(dir.path());
    seed_library(
        dir.path(),
        &[
            SeedTrack {
                file: "airbag.wav",
                title: "Airbag",
                artist: "Radiohead",
                album: "OK Computer",
                year: 1997,
                track_no: 1,
            },
            SeedTrack {
                file: "karma.wav",
                title: "Karma Police",
                artist: "Radiohead",
                album: "OK Computer",
                year: 1997,
                track_no: 6,
            },
            SeedTrack {
                file: "roads.wav",
                title: "Roads",
                artist: "Portishead",
                album: "Dummy",
                year: 1994,
                track_no: 1,
            },
        ],
    );
    env.set("MUSICWASP_TEST", Some("1"));

    let mut harness = harness();
    harness.run_steps(3);

    assert!(harness.query_by_label("Main menu").is_some());
    assert!(harness.query_by_label("Saved searches").is_some());

    // Tab should start at Search, the first focusable control in the toolbar.
    harness.key_press(egui::Key::Tab);
    harness.run_steps(1);
    let focused = text_inputs(&harness)
        .into_iter()
        .find(|node| node.is_focused())
        .expect("Tab should focus a text input");
    assert!(
        focused.value().is_none_or(|value| value.is_empty()),
        "the first tab stop should be the empty search box"
    );

    let inputs = text_inputs(&harness);
    let search = inputs.first().expect("search text input");
    search.focus();
    harness.run_steps(1);

    let inputs = text_inputs(&harness);
    inputs
        .first()
        .expect("search text input")
        .type_text("artist:\"radiohead\" year:1997");
    harness.run_steps(2);

    assert!(harness.query_by_label_contains("Airbag").is_some());
    assert!(harness.query_by_label_contains("Karma Police").is_some());
    assert!(
        harness.query_by_label_contains("Roads").is_none(),
        "the year/artist filter must remove the other track"
    );

    harness.state_mut().show_columns = true;
    harness.run_steps(2);
    assert!(harness.query_by_label("Track columns").is_some());

    harness
        .get_by_role_and_label(Role::CheckBox, "Artist")
        .click();
    harness.run_steps(1);
    assert!(harness
        .state()
        .settings
        .columns
        .is_hidden(TrackColumn::Artist));

    harness.get_by_label("Album art").click();
    harness.run_steps(1);
    assert_eq!(
        harness.state().settings.columns.layout,
        TrackLayout::AlbumArt
    );

    harness.get_by_label("Compact").click();
    harness.run_steps(1);
    assert_eq!(
        harness.state().settings.columns.layout,
        TrackLayout::Compact
    );

    harness.get_by_label("Main menu").click_accesskit();
    harness.run_steps(1);
    harness.get_by_label_contains("Settings").click_accesskit();
    harness.run_steps(1);
    assert!(
        harness.state().show_settings,
        "settings window did not open"
    );
    assert!(harness.query_by_label("Scan").is_some());
    assert!(harness.query_by_label("Library").is_some());
    harness
        .get_by_role_and_label(Role::CheckBox, "Larger text and controls")
        .click_accesskit();
    harness.run_steps(1);
    assert!(harness.state().settings.large_fonts);

    let ctx = harness.ctx.clone();
    harness.state_mut().set_skin(&ctx, Skin::HIGH_CONTRAST);
    harness.run_steps(1);
    assert_eq!(harness.state().settings.skin, Skin::HIGH_CONTRAST);

    harness.state_mut().set_language(Language::French);
    harness.run_steps(1);
    assert!(harness.query_by_label("Scanner").is_some());
    assert!(harness.query_by_label("Bibliothèque").is_some());
    harness.state_mut().set_language(Language::English);
    harness.run_steps(1);
}

#[test]
fn autotag_dialog_uses_results_from_the_fake_musicbrainz_server() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(dir.path());
    let fake = FakeMusicBrainz::start();
    let caa_api = fake.caa_api();
    env.set("MUSICWASP_MUSICBRAINZ_API", Some(fake.api.as_str()));
    env.set("MUSICWASP_COVERART_API", Some(caa_api.as_str()));
    seed_library(
        dir.path(),
        &[
            SeedTrack {
                file: "one.wav",
                title: "Track 1",
                artist: "Radiohead",
                album: "Fake Album",
                year: 1997,
                track_no: 1,
            },
            SeedTrack {
                file: "two.wav",
                title: "Track 2",
                artist: "Radiohead",
                album: "Fake Album",
                year: 1997,
                track_no: 2,
            },
        ],
    );

    let mut harness = harness();
    harness.run_steps(3);
    harness.state_mut().select_all_in_view();
    harness.state_mut().open_autotag();
    harness.run_steps(2);
    assert!(harness.query_by_label("Auto-tag by album").is_some());

    harness.get_by_label("Search").click();
    let finished = wait_until(&mut harness, Duration::from_secs(5), |app| {
        let Some(state) = &app.autotag else {
            return false;
        };
        matches!(state.stage, AutoTagStage::Results(_)) && state.release.is_some()
    });
    assert!(
        finished,
        "fake MusicBrainz search and release fetch timed out"
    );

    let state = harness.state().autotag.as_ref().unwrap();
    let release = state.release.as_ref().unwrap();
    assert_eq!(release.title, "Fake Album");
    assert_eq!(release.tracks.len(), 2);
    assert_eq!(release.tracks[0].title, "Airbag");

    let requests = fake.requests();
    assert!(
        requests.iter().any(|path| path.contains("/ws/2/release?")),
        "search request was not sent: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|path| path.contains("/ws/2/release/fake-release")),
        "release lookup was not sent: {requests:?}"
    );

    harness.run_steps(2);
    assert!(harness.query_all_by_label_contains("Fake Album").count() >= 1);
    assert!(harness.query_all_by_label_contains("Airbag").count() >= 1);
}
