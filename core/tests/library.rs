use musicwasp_core::{library, tags, Filter, Library, SortKey, TagEdit};
use std::io::Write;

/// Minimal PCM WAV so the test needs no sample files.
fn write_wav(path: &std::path::Path, frames: u32) {
    let sample_rate = 44_100u32;
    let data_len = frames; // 8-bit mono: 1 byte per frame
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap(); // fmt chunk size
    f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    f.write_all(&1u16.to_le_bytes()).unwrap(); // mono
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap(); // byte rate
    f.write_all(&1u16.to_le_bytes()).unwrap(); // block align
    f.write_all(&8u16.to_le_bytes()).unwrap(); // bits per sample
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(&vec![0u8; data_len as usize]).unwrap();
}

#[test]
fn scan_then_edit_tags_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let track = music.join("clip.wav");
    write_wav(&track, 44_100);

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    let n = lib.scan(&music, false).unwrap();
    assert_eq!(n, 1);

    let found = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "clip");
    assert_eq!(found[0].artist, "Unknown Artist");
    let duration = found[0].duration_secs;

    let mut edit = TagEdit::from_track(&found[0]);
    edit.title = "Real Title".into();
    edit.artist = "Some Artist".into();
    edit.album = "Some Album".into();
    edit.genre = "Ambient".into();
    edit.year = Some(2001);
    edit.track_no = Some(3);

    tags::write(&track, &edit).unwrap();
    lib.refresh_file(&track).unwrap();

    let after = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    assert_eq!(after.len(), 1, "edit must not duplicate the row");
    assert_eq!(after[0].title, "Real Title");
    assert_eq!(after[0].artist, "Some Artist");
    assert_eq!(after[0].album, "Some Album");
    assert_eq!(after[0].genre, "Ambient");
    assert_eq!(after[0].year, Some(2001));
    assert_eq!(after[0].track_no, Some(3));
    assert_eq!(after[0].duration_secs, duration);
}

#[test]
fn import_paths_indexes_dropped_files_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();

    let loose = dir.path().join("loose.wav");
    write_wav(&loose, 4410);
    let folder = dir.path().join("dropped");
    std::fs::create_dir_all(&folder).unwrap();
    write_wav(&folder.join("inside.wav"), 4410);
    std::fs::write(folder.join("notes.txt"), "not audio").unwrap();

    let indexed = lib.import_paths(&[loose.clone(), folder.clone()]).unwrap();
    assert_eq!(indexed.len(), 2);
    assert!(indexed.contains(&loose));
    assert_eq!(
        lib.query(&Filter::default(), SortKey::Title, true)
            .unwrap()
            .len(),
        2
    );

    // Dropping the same paths again updates rows instead of duplicating them.
    lib.import_paths(&[loose, folder]).unwrap();
    assert_eq!(
        lib.query(&Filter::default(), SortKey::Title, true)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn filter_and_column_queries() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let a = music.join("a.wav");
    let b = music.join("b.wav");
    write_wav(&a, 1000);
    write_wav(&b, 1000);

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();

    let mut edit =
        TagEdit::from_track(&lib.query(&Filter::default(), SortKey::Title, true).unwrap()[0]);
    edit.artist = "Alpha".into();
    edit.album = "First".into();
    edit.genre = "Rock".into();
    tags::write(&a, &edit).unwrap();
    lib.refresh_file(&a).unwrap();

    let rock = Filter {
        genre: Some("Rock".into()),
        ..Default::default()
    };
    assert_eq!(lib.query(&rock, SortKey::Title, true).unwrap().len(), 1);
    assert_eq!(lib.genres().unwrap(), vec!["Rock".to_string()]);
    assert!(lib
        .artists(Some("Rock"))
        .unwrap()
        .contains(&"Alpha".to_string()));
    assert!(lib
        .albums(Some("Alpha"), Some("Rock"))
        .unwrap()
        .contains(&"First".to_string()));

    let text = Filter {
        text: "alph".into(),
        ..Default::default()
    };
    assert_eq!(lib.query(&text, SortKey::Title, true).unwrap().len(), 1);

    let s = lib.stats().unwrap();
    assert_eq!(s.tracks, 2);
    assert_eq!(s.albums, 2, "one edited album plus the untouched default");
}

#[test]
fn rescan_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    write_wav(&music.join("x.wav"), 500);

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    lib.scan(&music, true).unwrap();

    assert_eq!(lib.stats().unwrap().tracks, 1);
    assert!(library::AUDIO_EXTENSIONS.contains(&"flac"));
}

#[test]
fn batch_edit_only_touches_filled_fields() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let a = music.join("a.wav");
    let b = music.join("b.wav");
    write_wav(&a, 800);
    write_wav(&b, 800);

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();

    // Give each file a distinct starting point.
    let first = TagEdit {
        title: "First".into(),
        artist: "Artist One".into(),
        album: "Album One".into(),
        genre: "Rock".into(),
        ..Default::default()
    };
    tags::write(&a, &first).unwrap();

    let second = TagEdit {
        title: "Second".into(),
        artist: "Artist Two".into(),
        album: "Album Two".into(),
        genre: "Jazz".into(),
        ..Default::default()
    };
    tags::write(&b, &second).unwrap();
    lib.refresh_file(&a).unwrap();
    lib.refresh_file(&b).unwrap();

    // Batch update: only the album and year change.
    let patch = TagEdit {
        album: "Shared Album".into(),
        year: Some(1999),
        ..Default::default()
    };
    tags::write_fields(&a, &patch, true).unwrap();
    tags::write_fields(&b, &patch, true).unwrap();
    lib.refresh_file(&a).unwrap();
    lib.refresh_file(&b).unwrap();

    let tracks = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    assert_eq!(tracks.len(), 2);
    let one = tracks.iter().find(|t| t.title == "First").unwrap();
    let two = tracks.iter().find(|t| t.title == "Second").unwrap();

    assert_eq!(one.album, "Shared Album");
    assert_eq!(two.album, "Shared Album");
    assert_eq!(one.year, Some(1999));
    assert_eq!(two.year, Some(1999));

    // Untouched fields survive the batch edit.
    assert_eq!(one.artist, "Artist One");
    assert_eq!(two.artist, "Artist Two");
    assert_eq!(one.genre, "Rock");
    assert_eq!(two.genre, "Jazz");
    assert_eq!(two.title, "Second");
}
