use musicwasp_core::{backup, tags, Filter, Library, SortKey, TagEdit};
use std::io::Write;
use std::path::Path;

fn write_wav(path: &Path) {
    let frames = 4_410u32;
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

fn add_track(music: &Path, file: &str, title: &str) {
    let path = music.join(file);
    write_wav(&path);
    tags::write(
        &path,
        &TagEdit {
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
}

#[test]
fn backup_restore_round_trip_keeps_a_safety_copy() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MUSICWASP_DATA_DIR", dir.path().join("data"));
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    add_track(&music, "one.wav", "One");

    let db = dir.path().join("library.db");
    let mut lib = Library::open_at(&db).unwrap();
    lib.scan(&music, false).unwrap();

    let backup = lib.create_backup().unwrap();
    assert!(backup.is_file());
    assert_eq!(backup::list().unwrap().len(), 1);
    backup::validate(&backup).unwrap();

    add_track(&music, "two.wav", "Two");
    lib.scan(&music, false).unwrap();
    assert_eq!(
        lib.query(&Filter::default(), SortKey::Title, true)
            .unwrap()
            .len(),
        2
    );

    let safety = lib.restore_from(&backup).unwrap();
    assert!(safety.is_file());
    let tracks = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    assert_eq!(tracks.len(), 1, "the pre-backup library was restored");
    assert_eq!(tracks[0].title, "One");

    // The restored connection is fully usable.
    lib.scan(&music, false).unwrap();
    assert_eq!(
        lib.query(&Filter::default(), SortKey::Title, true)
            .unwrap()
            .len(),
        2
    );

    let fake = dir.path().join("fake.db");
    std::fs::write(&fake, b"not sqlite").unwrap();
    assert!(backup::validate(&fake).is_err());
}
