use musicwasp_core::{tags, Filter, Library, SortKey, TagEdit};
use std::io::Write;

/// Minimal PCM WAV so the test needs no sample files.
fn write_wav(path: &std::path::Path, frames: u32) {
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

fn tag(path: &std::path::Path, title: &str, artist: &str, album: &str) {
    tags::write(
        path,
        &TagEdit {
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            ..Default::default()
        },
    )
    .unwrap();
}

#[test]
fn hygiene_finds_missing_and_duplicates_and_deletes_rows_only() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();

    let first = music.join("first.wav");
    let second = music.join("second.wav");
    let gone = music.join("gone.wav");
    for path in [&first, &second, &gone] {
        write_wav(path, 4_410);
    }
    tag(&first, "Same Song", "Same Artist", "Same Album");
    tag(&second, "same song ", "same  artist", "SAME ALBUM");
    tag(&gone, "Vanishing", "Other Artist", "Other Album");

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    assert_eq!(lib.scan(&music, false).unwrap(), 3);
    let all = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    let gone_id = all
        .iter()
        .find(|track| track.path.ends_with("gone.wav"))
        .unwrap()
        .id;

    // A file that disappeared after indexing is reported, nothing else.
    std::fs::remove_file(&gone).unwrap();
    let missing = lib.missing_tracks().unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].id, gone_id);

    // Case and whitespace differences still count as the same recording.
    let groups = lib.duplicate_groups().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 2);
    assert!(groups[0][0].path.ends_with("first.wav"));
    assert!(groups[0][1].path.ends_with("second.wav"));

    // Removing rows must not touch the audio files.
    let ids: Vec<i64> = all.iter().map(|track| track.id).collect();
    let playlist = lib.create_playlist("Everything", &ids).unwrap();
    assert_eq!(lib.playlists().unwrap()[0].count, 3);

    let removed = lib
        .remove_tracks(&[groups[0][0].id, groups[0][1].id])
        .unwrap();
    assert_eq!(removed, 2);
    assert!(first.exists());
    assert!(second.exists());
    assert_eq!(
        lib.query(&Filter::default(), SortKey::Title, true)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lib.playlist_tracks(playlist).unwrap().len(),
        1,
        "playlist rows cascade with the deleted tracks"
    );

    assert_eq!(lib.remove_tracks(&[]).unwrap(), 0);
    assert_eq!(lib.remove_tracks(&[gone_id]).unwrap(), 1);
    assert!(lib.missing_tracks().unwrap().is_empty());
}
