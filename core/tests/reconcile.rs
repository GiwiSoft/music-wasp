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

fn tag(path: &std::path::Path, title: &str) {
    tags::write(
        path,
        &TagEdit {
            title: title.to_string(),
            artist: "Same Artist".to_string(),
            album: "Same Album".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
}

fn track_by_path(lib: &Library, suffix: &str) -> musicwasp_core::Track {
    lib.query(&Filter::default(), SortKey::Title, true)
        .unwrap()
        .into_iter()
        .find(|track| track.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no track ending with {suffix}"))
}

#[test]
fn renamed_file_keeps_its_row_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let first = music.join("first.wav");
    let second = music.join("second.wav");
    write_wav(&first, 44_100);
    write_wav(&second, 88_200);
    tag(&first, "First Song");
    tag(&second, "Second Song");

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    let old = track_by_path(&lib, "first.wav");
    lib.set_rating(old.id, 5).unwrap();
    let playlist = lib.create_playlist("Keep", &[old.id]).unwrap();

    let renamed = music.join("renamed.wav");
    std::fs::rename(&first, &renamed).unwrap();

    // The preview is read-only.
    let plan = lib.reconcile(std::slice::from_ref(&music)).unwrap();
    assert_eq!(plan.missing.len(), 1);
    assert_eq!(plan.candidates.len(), 1);
    assert!(plan.unmatched.is_empty());
    let candidate = &plan.candidates[0];
    assert_eq!(candidate.track.id, old.id);
    assert!(candidate.new_path.ends_with("renamed.wav"));
    assert!(candidate.replaces_id.is_none());
    assert!(candidate.reasons.iter().any(|reason| reason == "size"));
    assert!(candidate.reasons.iter().any(|reason| reason == "mtime"));
    assert_eq!(
        track_by_path(&lib, "first.wav").path,
        old.path,
        "preview must not move rows"
    );

    let outcome = lib.apply_reconcile(&plan.candidates).unwrap();
    assert_eq!(outcome.moved, 1);
    assert!(outcome.removed.is_empty());
    let moved = track_by_path(&lib, "renamed.wav");
    assert_eq!(moved.id, old.id, "ratings and history follow the row");
    assert_eq!(moved.rating, 5);
    assert!(lib.missing_tracks().unwrap().is_empty());
    assert_eq!(
        lib.playlist_tracks(playlist).unwrap().len(),
        1,
        "playlist entries follow the moved row"
    );
}

#[test]
fn watcher_reimported_file_is_merged_back() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let first = music.join("first.wav");
    let second = music.join("second.wav");
    write_wav(&first, 44_100);
    write_wav(&second, 88_200);
    tag(&first, "First Song");
    tag(&second, "Second Song");

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    let old = track_by_path(&lib, "first.wav");
    lib.set_rating(old.id, 4).unwrap();

    // Rename, then let the watcher import the new path as a new row.
    let renamed = music.join("renamed.wav");
    std::fs::rename(&first, &renamed).unwrap();
    lib.scan(&music, false).unwrap();
    assert_eq!(
        lib.query(&Filter::default(), SortKey::Title, true)
            .unwrap()
            .len(),
        3
    );
    let duplicate = track_by_path(&lib, "renamed.wav");

    let plan = lib.reconcile(std::slice::from_ref(&music)).unwrap();
    assert_eq!(plan.candidates.len(), 1);
    let candidate = &plan.candidates[0];
    assert_eq!(candidate.track.id, old.id);
    assert_eq!(candidate.replaces_id, Some(duplicate.id));

    let outcome = lib.apply_reconcile(&plan.candidates).unwrap();
    assert_eq!(outcome.moved, 1);
    assert_eq!(outcome.removed, vec![duplicate.id]);
    let all = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    assert_eq!(all.len(), 2, "the watcher duplicate was merged away");
    let kept = track_by_path(&lib, "renamed.wav");
    assert_eq!(kept.id, old.id);
    assert_eq!(kept.rating, 4);
}

#[test]
fn same_tags_but_different_file_are_not_merged() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let first = music.join("first.wav");
    let second = music.join("second.wav");
    write_wav(&first, 44_100);
    write_wav(&second, 176_400);
    tag(&first, "Same Song");
    tag(&second, "Same Song");

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();

    std::fs::remove_file(&first).unwrap();
    let plan = lib.reconcile(&[music]).unwrap();
    assert_eq!(plan.missing.len(), 1);
    assert!(
        plan.candidates.is_empty(),
        "an indexed lookalike is not merged without file evidence"
    );
    assert_eq!(plan.unmatched.len(), 1);
}
