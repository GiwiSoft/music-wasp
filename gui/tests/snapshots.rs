#![cfg(target_os = "linux")]

//! Image snapshots for the built-in skins and accessibility modes.
//!
//! Update baselines with `UPDATE_SNAPSHOTS=force cargo test -p musicwasp-gui --test snapshots`.

use egui_kittest::Harness;
use musicwasp_core::{tags, Library, TagEdit};
use musicwasp_gui::app::App;
use musicwasp_gui::theme::Skin;
use std::io::Write;
use std::path::Path;

fn write_wav(path: &Path, frames: u32) {
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + frames).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&44_100u32.to_le_bytes()).unwrap();
    file.write_all(&44_100u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&8u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&frames.to_le_bytes()).unwrap();
    file.write_all(&vec![0u8; frames as usize]).unwrap();
}

fn seed(root: &Path) {
    let music = root.join("music");
    std::fs::create_dir_all(&music).unwrap();
    for (file, title, artist, album, genre, year, track) in [
        (
            "01 - Airbag.wav",
            "Airbag",
            "Radiohead",
            "OK Computer",
            "Alternative",
            1997,
            1,
        ),
        (
            "02 - Roads.wav",
            "Roads",
            "Portishead",
            "Dummy",
            "Trip Hop",
            1994,
            7,
        ),
        (
            "03 - Teardrop.wav",
            "Teardrop",
            "Massive Attack",
            "Mezzanine",
            "Trip Hop",
            1998,
            3,
        ),
    ] {
        let path = music.join(file);
        write_wav(&path, 44_100);
        tags::write(
            &path,
            &TagEdit {
                title: title.into(),
                artist: artist.into(),
                album: album.into(),
                genre: genre.into(),
                year: Some(year),
                track_no: Some(track),
                ..Default::default()
            },
        )
        .unwrap();
    }
    Library::open_at(&root.join("library.db"))
        .unwrap()
        .scan(&music, false)
        .unwrap();
}

#[test]
fn main_skins_and_accessibility_states() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MUSICWASP_DATA_DIR", dir.path());
    std::env::set_var("MUSICWASP_TEST", "1");
    std::env::set_var("MUSICWASP_DEBUG", "1");
    seed(dir.path());

    let mut harness: Harness<'static, App> = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(|cc| App::new(cc).expect("app"));
    harness.run_steps(4);

    let states = [
        ("skins_wasp", Skin::WASP, false),
        ("skins_light", Skin::LIGHT, false),
        ("skins_high_contrast", Skin::HIGH_CONTRAST, false),
        ("skins_liquid_glass_dark", Skin::LIQUID_GLASS_DARK, false),
        (
            "skins_winamp_classic",
            Skin::from_id("winamp_classic"),
            false,
        ),
        ("skins_material_dark", Skin::from_id("material_dark"), false),
        (
            "skins_material_light",
            Skin::from_id("material_light"),
            false,
        ),
        ("skins_cosmic_dark", Skin::from_id("cosmic_dark"), false),
        ("skins_cosmic_light", Skin::from_id("cosmic_light"), false),
        (
            "skins_cupertino_dark",
            Skin::from_id("cupertino_dark"),
            false,
        ),
        (
            "skins_cupertino_light",
            Skin::from_id("cupertino_light"),
            false,
        ),
        ("accessibility_large_text", Skin::WASP, true),
        (
            "accessibility_high_contrast_large_text",
            Skin::HIGH_CONTRAST,
            true,
        ),
    ];

    for (name, skin, large_fonts) in states {
        let ctx = harness.ctx.clone();
        harness.state_mut().set_skin(&ctx, skin);
        harness.state_mut().set_large_fonts(&ctx, large_fonts);
        harness.run_steps(3);
        harness.snapshot(name);
    }
}
