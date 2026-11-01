use super::{
    apply_click, auto_number_tracks, best_release_track, clamp_seek, common_fields, list_subdirs,
    mapping_matches, normalize_case_value, queue_insert_next, queue_reorder,
    remove_tracks_from_queue, restore_queue, sanitize_file_name, suggested_mappings, BatchCase,
    BatchField, CoverCache, TrackField,
};
use crate::i18n::tr;
use musicwasp_core::autotag::{Release, ReleaseTrack};
use musicwasp_core::{TagEdit, Track};
use std::collections::BTreeSet;

#[test]
fn cover_cache_evicts_the_least_recently_used_album() {
    let mut cache = CoverCache::with_capacity(2);
    cache.insert("a".to_string(), None);
    cache.insert("b".to_string(), None);
    assert!(cache.get("a").is_some(), "a is now recently used");
    cache.insert("c".to_string(), None);

    assert!(cache.get("a").is_some());
    assert!(cache.get("b").is_none(), "b was least recently used");
    assert!(cache.get("c").is_some());
    assert_eq!(cache.len(), 2);

    cache.remove("a");
    assert_eq!(cache.len(), 1);
    cache.clear();
    assert_eq!(cache.len(), 0);
}

#[test]
fn m3u_export_names_are_portable() {
    assert_eq!(sanitize_file_name("Road trip"), "Road trip");
    assert_eq!(sanitize_file_name("AC/DC: live?"), "AC_DC_ live_");
    assert_eq!(sanitize_file_name("..."), "playlist");
    assert_eq!(sanitize_file_name("  "), "playlist");
}

#[test]
fn renaming_numeric_fields_validates_ranges() {
    let mut edit = TagEdit::default();
    TrackField::Year.apply(&mut edit, "1999").unwrap();
    assert_eq!(edit.year, Some(1999));
    TrackField::Year.apply(&mut edit, "").unwrap();
    assert_eq!(edit.year, None);
    assert!(TrackField::Year.apply(&mut edit, "nineteen").is_err());

    TrackField::TrackNo.apply(&mut edit, "1000").unwrap_err();
    TrackField::TrackNo.apply(&mut edit, "7").unwrap();
    assert_eq!(edit.track_no, Some(7));

    TrackField::Title.apply(&mut edit, "  Song  ").unwrap();
    assert_eq!(edit.title, "Song");
}

#[test]
fn relative_seek_stays_inside_the_track() {
    assert_eq!(clamp_seek(10.0, 5.0, 60.0), 15.0);
    assert_eq!(clamp_seek(10.0, -5.0, 60.0), 5.0);
    // Cannot rewind before the start.
    assert_eq!(clamp_seek(2.0, -5.0, 60.0), 0.0);
    // Forward past the end lands just short of it.
    assert!((clamp_seek(58.0, 5.0, 60.0) - 59.95).abs() < 1e-9);
    // Unknown duration: clamp only at zero.
    assert_eq!(clamp_seek(2.0, 5.0, 0.0), 7.0);
}

fn selection(ids: &[i64]) -> BTreeSet<i64> {
    ids.iter().copied().collect()
}

#[test]
fn plain_click_replaces_the_selection() {
    let view = [10, 20, 30];
    let mut selected = selection(&[10, 20]);
    let mut anchor = Some(10);

    apply_click(&mut selected, &mut anchor, &view, 2, false, false);
    assert_eq!(selected, selection(&[30]));
    assert_eq!(anchor, Some(30));
}

#[test]
fn ctrl_click_toggles_rows() {
    let view = [10, 20, 30];
    let mut selected = selection(&[10]);
    let mut anchor = None;

    apply_click(&mut selected, &mut anchor, &view, 1, true, false);
    assert_eq!(selected, selection(&[10, 20]));
    assert_eq!(anchor, Some(20));

    // Clicking the same row again removes it.
    apply_click(&mut selected, &mut anchor, &view, 1, true, false);
    assert_eq!(selected, selection(&[10]));
}

#[test]
fn shift_click_selects_a_range_from_the_anchor() {
    let view = [10, 20, 30, 40];
    let mut selected = selection(&[]);
    let mut anchor = None;

    apply_click(&mut selected, &mut anchor, &view, 1, false, false);
    apply_click(&mut selected, &mut anchor, &view, 3, false, true);
    assert_eq!(selected, selection(&[20, 30, 40]));

    // Backwards range too.
    apply_click(&mut selected, &mut anchor, &view, 0, false, true);
    assert_eq!(selected, selection(&[10, 20]));
}

#[test]
fn shift_click_without_anchor_falls_back_to_single_select() {
    let view = [10, 20];
    let mut selected = selection(&[]);
    let mut anchor = None;
    apply_click(&mut selected, &mut anchor, &view, 1, false, true);
    assert_eq!(selected, selection(&[20]));
    assert_eq!(anchor, Some(20));
}

#[test]
fn out_of_range_clicks_are_ignored() {
    let view = [10];
    let mut selected = selection(&[10]);
    let mut anchor = Some(10);
    apply_click(&mut selected, &mut anchor, &view, 7, false, false);
    assert_eq!(selected, selection(&[10]));
}

#[test]
fn queue_inserts_after_the_current_track_and_reorders_only_up_next() {
    let mut queue = vec![1, 2, 3];
    queue_insert_next(&mut queue, Some(0), &[8, 9]);
    assert_eq!(queue, vec![1, 8, 9, 2, 3]);

    queue_insert_next(&mut queue, None, &[7]);
    assert_eq!(queue, vec![1, 8, 9, 2, 3, 7]);

    // Reorder upcoming entries.
    assert!(queue_reorder(&mut queue, Some(0), 4, 1));
    assert_eq!(queue, vec![1, 3, 8, 9, 2, 7]);

    // Move to the very end.
    let end = queue.len();
    assert!(queue_reorder(&mut queue, Some(0), 2, end));
    assert_eq!(queue, vec![1, 3, 9, 2, 7, 8]);

    // Never move the playing entry or anything before it.
    assert!(!queue_reorder(&mut queue, Some(0), 0, 3));
    assert!(!queue_reorder(&mut queue, Some(2), 1, 4));
    assert_eq!(queue, vec![1, 3, 9, 2, 7, 8]);
}

#[test]
fn restored_queue_drops_deleted_tracks_and_keeps_the_playing_slot() {
    let valid = selection(&[1, 3, 9]);
    let (queue, pos) = restore_queue(&[1, 2, 3, 9, 1], Some(3), &valid);
    assert_eq!(queue, vec![1, 3, 9, 1]);
    assert_eq!(pos, Some(2), "the third entry is still the current one");

    // If the current track was deleted, do not start some other track.
    let (queue, pos) = restore_queue(&[1, 2, 3], Some(1), &valid);
    assert_eq!(queue, vec![1, 3]);
    assert_eq!(pos, None);

    // Duplicate ids keep their position.
    let (queue, pos) = restore_queue(&[9, 1, 9], Some(2), &valid);
    assert_eq!(queue, vec![9, 1, 9]);
    assert_eq!(pos, Some(2));
}

#[test]
fn removing_library_tracks_stops_only_if_the_current_entry_is_gone() {
    let removed = selection(&[2]);

    let mut queue = vec![1, 2, 3, 4];
    let (pos, current_removed) = remove_tracks_from_queue(&mut queue, Some(2), &removed);
    assert_eq!(queue, vec![1, 3, 4]);
    assert_eq!(pos, Some(1));
    assert!(!current_removed);

    let mut queue = vec![1, 2, 4];
    let (pos, current_removed) = remove_tracks_from_queue(&mut queue, Some(1), &removed);
    assert_eq!(queue, vec![1, 4]);
    assert_eq!(pos, None);
    assert!(current_removed);
}

fn track(title: &str, artist: &str, album: &str, year: Option<i32>) -> Track {
    Track {
        id: 0,
        path: String::new(),
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        album_artist: String::new(),
        genre: String::new(),
        year,
        track_no: None,
        disc_no: None,
        duration_secs: 0,
        sample_rate: 0,
        bitrate: 0,
        format: String::new(),
    }
}

fn release_track(disc: u32, number: u32, title: &str) -> ReleaseTrack {
    ReleaseTrack {
        disc,
        number,
        title: title.into(),
        artist: tr("Artist").into(),
        length_ms: Some(200_000),
    }
}

#[test]
fn single_file_tagging_picks_the_right_release_track() {
    let release = Release {
        id: "r".into(),
        title: "Album".into(),
        artist: tr("Artist").into(),
        album_artist: tr("Artist").into(),
        year: None,
        genre: None,
        tracks: vec![
            release_track(1, 1, "First Song"),
            release_track(1, 2, "Second Song"),
            release_track(2, 1, "Third Song"),
        ],
        has_front_cover: false,
    };

    let mut local = track("Wrong local title", tr("Artist"), "Album", None);
    local.track_no = Some(2);
    local.disc_no = Some(1);
    assert_eq!(best_release_track(&local, &release), 1);

    // No usable number: fall back to an exact title match.
    local.track_no = None;
    local.title = "Third Song".into();
    assert_eq!(best_release_track(&local, &release), 2);

    // Nothing matches: closest duration, here all equal, so the first.
    local.title = "Unknown".into();
    assert_eq!(best_release_track(&local, &release), 0);
}

#[test]
fn album_preview_maps_by_track_number_and_exposes_diffs() {
    let release = Release {
        id: "r".into(),
        title: "Album".into(),
        artist: tr("Artist").into(),
        album_artist: tr("Artist").into(),
        year: Some(2000),
        genre: None,
        tracks: vec![
            release_track(1, 1, "First Song"),
            release_track(1, 2, "Second Song"),
        ],
        has_front_cover: false,
    };

    let mut second = track("Wrong two", tr("Artist"), "Album", None);
    second.track_no = Some(2);
    let mut first = track("Wrong one", tr("Artist"), "Album", None);
    first.track_no = Some(1);

    let mut mappings = suggested_mappings(&[second, first], &release);
    assert_eq!(mappings[0].release_index, Some(1));
    assert_eq!(mappings[1].release_index, Some(0));

    let changes = mappings[1].changes(&release);
    assert!(changes
        .iter()
        .any(|(field, _, new)| *field == tr("Title") && new == "First Song"));

    // A manual override changes what gets written; skip removes the row.
    mappings[0].release_index = Some(0);
    mappings[1].release_index = None;
    let matches = mapping_matches(&mappings, &release);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].track.title, "First Song");
}

#[test]
fn batch_transforms_number_per_disc_and_normalize_case() {
    let mut edits = vec![
        TagEdit {
            title: "hELLO wORLD".into(),
            disc_no: Some(1),
            ..Default::default()
        },
        TagEdit {
            disc_no: Some(1),
            ..Default::default()
        },
        TagEdit {
            disc_no: Some(2),
            ..Default::default()
        },
    ];
    auto_number_tracks(&mut edits);
    assert_eq!(edits[0].track_no, Some(1));
    assert_eq!(edits[1].track_no, Some(2));
    assert_eq!(edits[2].track_no, Some(1), "numbering restarts per disc");

    assert_eq!(
        normalize_case_value("hELLO wORLD", BatchCase::Title),
        "Hello World"
    );
    assert_eq!(
        normalize_case_value("hELLO wORLD", BatchCase::Lower),
        "hello world"
    );
    assert_eq!(
        normalize_case_value("hELLO wORLD", BatchCase::Upper),
        "HELLO WORLD"
    );
}

#[test]
fn batch_field_copy_and_clear_round_trip() {
    let source = TagEdit {
        genre: "Alternative".into(),
        year: Some(1997),
        ..Default::default()
    };
    let mut target = TagEdit::default();
    BatchField::Genre.set(&mut target, &BatchField::Genre.value(&source));
    BatchField::Year.set(&mut target, &BatchField::Year.value(&source));
    assert_eq!(target.genre, "Alternative");
    assert_eq!(target.year, Some(1997));

    BatchField::Genre.clear(&mut target);
    BatchField::Year.clear(&mut target);
    assert!(target.genre.is_empty());
    assert_eq!(target.year, None);
}

#[test]
fn batch_prefill_keeps_shared_fields_and_blanks_differing_ones() {
    let tracks = [
        track("One", "Same Artist", "Album A", Some(2001)),
        track("Two", "Same Artist", "Album B", Some(2002)),
    ];
    let common = common_fields(&tracks);

    assert_eq!(common.artist, "Same Artist", "shared value is prefilled");
    assert_eq!(common.title, "", "differing values stay blank");
    assert_eq!(common.album, "");
    assert_eq!(common.year, None);
    assert!(!common.is_empty());
}

#[test]
fn folder_listing_hides_dot_directories() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Music")).unwrap();
    std::fs::create_dir(dir.path().join(".hidden")).unwrap();
    std::fs::write(dir.path().join("file.mp3"), b"x").unwrap();

    let visible = list_subdirs(dir.path(), false).unwrap();
    assert_eq!(visible.len(), 1);
    assert!(visible[0].ends_with("Music"));

    let all = list_subdirs(dir.path(), true).unwrap();
    assert_eq!(all.len(), 2);
}
