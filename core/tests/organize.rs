use musicwasp_core::organize::{self, MoveStatus};
use musicwasp_core::{tags, Filter, Library, SortKey, TagEdit};
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

fn add_track(music: &Path, file: &str, title: &str, artist: &str, track: u32) {
    let path = music.join(file);
    write_wav(&path, 4_410);
    tags::write(
        &path,
        &TagEdit {
            title: title.to_string(),
            artist: artist.to_string(),
            album: "Album".to_string(),
            album_artist: artist.to_string(),
            track_no: Some(track as i32),
            year: Some(2026),
            ..Default::default()
        },
    )
    .unwrap();
}

fn scan(music: &Path, lib: &Library) -> Vec<musicwasp_core::Track> {
    lib.scan(music, false).unwrap();
    lib.query(&Filter::default(), SortKey::Artist, true)
        .unwrap()
}

const PATTERN: &str = "{artist}/{album}/{track:02} - {title}";

#[test]
fn preview_moves_files_and_undo_restores_them() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    add_track(&music, "a.wav", "Airbag", "Radiohead", 1);
    add_track(&music, "b.wav", "Roads", "Portishead", 7);

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    let tracks = scan(&music, &lib);

    let plan = organize::plan(&tracks, &music, PATTERN).unwrap();
    assert_eq!(plan.ready, 2);
    assert_eq!(plan.skipped, 0);
    let airbag = plan.rows.iter().find(|row| row.title == "Airbag").unwrap();
    assert_eq!(
        airbag.target_path,
        music.join("Radiohead/Album/01 - Airbag.wav")
    );

    let report = lib.organize_apply(&plan.rows).unwrap();
    assert_eq!(report.moved, 2);
    assert!(report.failed.is_empty());
    assert!(!music.join("a.wav").exists());
    assert!(music.join("Radiohead/Album/01 - Airbag.wav").is_file());
    assert!(lib.missing_tracks().unwrap().is_empty());
    let moved = lib
        .query(
            &Filter {
                text: "airbag".into(),
                ..Default::default()
            },
            SortKey::Title,
            true,
        )
        .unwrap();
    assert_eq!(
        std::path::Path::new(&moved[0].path),
        music.join("Radiohead/Album/01 - Airbag.wav"),
        "the library row follows the file into its new folder"
    );

    // A second preview of the same layout has nothing left to do.
    let tracks = lib
        .query(&Filter::default(), SortKey::Artist, true)
        .unwrap();
    let plan = organize::plan(&tracks, &music, PATTERN).unwrap();
    assert_eq!(plan.ready, 0);
    assert_eq!(plan.skipped, 2);
    assert!(plan
        .rows
        .iter()
        .all(|row| row.status == MoveStatus::Unchanged));

    let undo = lib.organize_undo(&report.records).unwrap();
    assert_eq!(undo.restored, 2);
    assert!(undo.failed.is_empty());
    assert!(music.join("a.wav").is_file());
    assert!(music.join("b.wav").is_file());
    assert!(!music.join("Radiohead/Album/01 - Airbag.wav").exists());
    assert!(lib.missing_tracks().unwrap().is_empty());
}

#[test]
fn collisions_and_bad_patterns_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    add_track(&music, "one.wav", "Same", "Artist", 1);
    add_track(&music, "two.wav", "Same", "Artist", 1);
    add_track(&music, "three.wav", "Other", "Artist", 2);

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    let tracks = scan(&music, &lib);

    let plan = organize::plan(&tracks, &music, PATTERN).unwrap();
    let duplicates: Vec<_> = plan.rows.iter().filter(|row| row.title == "Same").collect();
    assert_eq!(duplicates.len(), 2);
    assert!(duplicates
        .iter()
        .all(|row| row.status == MoveStatus::Collision));
    assert_eq!(plan.ready, 1, "the unique track can still move");

    // An existing file at the target blocks the move.
    let blocked = music.join("Artist/Album/02 - Other.wav");
    std::fs::create_dir_all(blocked.parent().unwrap()).unwrap();
    std::fs::write(&blocked, b"occupied").unwrap();
    let plan = organize::plan(&tracks, &music, PATTERN).unwrap();
    let other = plan.rows.iter().find(|row| row.title == "Other").unwrap();
    assert_eq!(other.status, MoveStatus::Collision);

    // Invalid placeholders become invalid rows instead of panicking.
    let plan = organize::plan(&tracks, &music, "{artist}/{nope}").unwrap();
    assert!(plan
        .rows
        .iter()
        .all(|row| row.status == MoveStatus::Invalid));
    assert!(plan.rows[0].detail.contains("nope"));
}

#[test]
fn move_journal_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moves-undo.json");
    let records = vec![organize::MoveRecord {
        track_id: 3,
        from: dir.path().join("old.flac"),
        to: dir.path().join("new.flac"),
    }];
    organize::save_undo(&path, &records).unwrap();
    let loaded = organize::load_undo(&path).unwrap().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].track_id, 3);
    assert_eq!(loaded[0].to, records[0].to);
    organize::clear_undo(&path);
    assert!(organize::load_undo(&path).unwrap().is_none());
}

#[test]
fn target_pattern_pads_numbers_and_sanitizes_names() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    add_track(&music, "x.wav", "AC/DC: Live?", "Artist", 3);
    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    let tracks = scan(&music, &lib);
    let target = organize::target_for(&music, &tracks[0], "{artist}/{track:03} {title}").unwrap();
    assert_eq!(
        target,
        music.join("Artist/003 AC_DC_ Live_.wav"),
        "slashes and colons cannot create extra folders"
    );
}
