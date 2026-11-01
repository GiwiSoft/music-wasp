use musicwasp_core::smart::{Definition, Field, Op, Rule};
use musicwasp_core::{db, Filter, Library, SortKey, TagEdit};
use std::io::Write;

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

#[test]
fn ratings_play_counts_skips_and_history_filters() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    for (name, title) in [("one.wav", "One"), ("two.wav", "Two")] {
        let path = music.join(name);
        write_wav(&path, 400);
        musicwasp_core::tags::write(
            &path,
            &TagEdit {
                title: title.into(),
                artist: "Artist".into(),
                album: "Album".into(),
                ..Default::default()
            },
        )
        .unwrap();
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    let tracks = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(tracks.iter().all(|track| track.date_added > 0));
    let first = tracks[0].id;

    lib.set_rating(first, 4).unwrap();
    lib.record_play(first).unwrap();
    lib.record_skip(first).unwrap();

    let stored = lib
        .query(&Filter::default(), SortKey::Rating, false)
        .unwrap()
        .into_iter()
        .find(|track| track.id == first)
        .unwrap();
    assert_eq!(stored.rating, 4);
    assert_eq!(stored.play_count, 1);
    assert_eq!(stored.skip_count, 1);
    assert!(stored.last_played.is_some());

    let never = Filter {
        never_played: true,
        ..Default::default()
    };
    let unplayed = lib.query(&never, SortKey::Title, true).unwrap();
    assert_eq!(unplayed.len(), 1);
    assert_ne!(unplayed[0].id, first);

    let conn = db::open_at(&dir.path().join("library.db")).unwrap();
    conn.execute(
        "UPDATE tracks SET last_played = unixepoch() - ?1 WHERE id = ?2",
        rusqlite::params![100 * 86_400i64, first],
    )
    .unwrap();
    let stale = Filter {
        not_played_days: Some(90),
        ..Default::default()
    };
    assert_eq!(lib.query(&stale, SortKey::Title, true).unwrap().len(), 2);

    let definition = Definition {
        match_all: true,
        rules: vec![
            Rule {
                field: Field::Rating,
                op: Op::Greater,
                value: "3".into(),
            },
            Rule {
                field: Field::PlayCount,
                op: Op::Greater,
                value: "0".into(),
            },
        ],
        ..Default::default()
    };
    let smart = lib.evaluate_smart(&definition).unwrap();
    assert_eq!(smart.len(), 1);
    assert_eq!(smart[0].id, first);

    // The migration creates the statistics columns for an older database.
    let conn = db::open_at(&dir.path().join("library.db")).unwrap();
    for column in [
        "rating",
        "play_count",
        "skip_count",
        "last_played",
        "date_added",
    ] {
        let found: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name = '{column}'"
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "missing tracks.{column}");
    }
}
