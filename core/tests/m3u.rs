use musicwasp_core::{m3u, Filter, Library, SortKey};
use std::io::Write;
use std::path::Path;

fn write_wav(path: &Path, frames: u32) {
    let sample_rate = 44_100u32;
    let data_len = frames;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&8u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(&vec![0u8; data_len as usize]).unwrap();
}

#[test]
fn import_indexes_relative_files_skips_missing_and_keeps_order() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    write_wav(&music.join("one.wav"), 1_000);
    write_wav(&music.join("two.wav"), 1_000);
    let playlist_path = dir.path().join("Mix.m3u8");

    let text = "\u{feff}#EXTM3U\n\
                #EXTINF:2,Artist - Two\n\
                music/two.wav\n\
                #EXTINF:-1,Artist - One\n\
                music/one.wav\n\
                music/missing.wav\n\
                https://radio.example/stream\n";
    std::fs::write(&playlist_path, text).unwrap();

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    let report = lib.import_m3u(&playlist_path, None, None).unwrap();
    assert_eq!(report.name, "Mix");
    assert_eq!(report.added, 2);
    assert_eq!(report.missing, 1);
    assert_eq!(report.unsupported, 1);

    let tracks = lib.playlist_tracks(report.playlist_id).unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(tracks[0].path.ends_with("two.wav"));
    assert!(tracks[1].path.ends_with("one.wav"));

    let listed = lib.playlists().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Mix");
    assert_eq!(listed[0].count, 2);
}

#[test]
fn export_round_trip_uses_absolute_paths_and_extinf() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    for name in ["a.wav", "b.wav"] {
        write_wav(&music.join(name), 500);
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    let ids: Vec<i64> = lib
        .query(&Filter::default(), SortKey::Title, true)
        .unwrap()
        .iter()
        .map(|track| track.id)
        .collect();
    let playlist_id = lib.create_playlist("Road trip", &ids).unwrap();

    let exported = dir.path().join("roadtrip.m3u8");
    assert_eq!(lib.export_m3u(playlist_id, &exported).unwrap(), 2);
    let text = std::fs::read_to_string(&exported).unwrap();
    assert!(text.starts_with("#EXTM3U\n"));
    assert!(text.contains("#PLAYLIST:Road trip\n"));
    assert!(text.contains(&music.join("a.wav").to_string_lossy().to_string()));

    // Re-importing the exported file produces the same tracks and order.
    let report = lib
        .import_m3u(&exported, Some("Imported trip"), None)
        .unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(report.missing, 0);
    let imported = lib.playlist_tracks(report.playlist_id).unwrap();
    let imported_ids: Vec<i64> = imported.iter().map(|track| track.id).collect();
    assert_eq!(imported_ids, ids);
}

#[test]
fn import_can_place_the_new_playlist_in_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    write_wav(&music.join("song.wav"), 500);

    let playlist_path = dir.path().join("One.m3u");
    std::fs::write(&playlist_path, "music/song.wav\n").unwrap();

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    let folder = lib.create_playlist_folder("Imported", None).unwrap();
    let report = lib.import_m3u(&playlist_path, None, Some(folder)).unwrap();
    assert_eq!(report.added, 1);
    assert_eq!(lib.playlists().unwrap()[0].folder_id, Some(folder));
}

#[test]
fn recognizes_playlist_extensions() {
    assert!(m3u::is_m3u_path(Path::new("mix.m3u")));
    assert!(m3u::is_m3u_path(Path::new("mix.M3U8")));
    assert!(!m3u::is_m3u_path(Path::new("song.mp3")));
}
