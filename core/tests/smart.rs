use musicwasp_core::smart::{self, Definition, Field, Op, Rule, SortBy};
use musicwasp_core::{Filter, Library, SortKey, TagEdit};
use std::io::Write;

fn write_wav(path: &std::path::Path, frames: u32) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + frames).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&44_100u32.to_le_bytes()).unwrap();
    f.write_all(&44_100u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&8u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&frames.to_le_bytes()).unwrap();
    f.write_all(&vec![0u8; frames as usize]).unwrap();
}

struct Album {
    lib: Library,
    _dir: tempfile::TempDir,
}

fn library_with_tracks() -> Album {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();

    // title, artist, genre, year, whole seconds of audio
    let rows = [
        ("First", "Alpha", "Rock", 1995, 1u32),
        ("Second", "Beta", "Jazz", 2005, 3),
        ("Third", "Alpha", "Rock", 2015, 2),
        ("Fourth", "Gamma", "Ambient", 1985, 4),
    ];

    for (title, artist, genre, year, secs) in rows {
        let path = music.join(format!("{title}.wav"));
        write_wav(&path, secs * 44_100);
        let edit = TagEdit {
            title: title.into(),
            artist: artist.into(),
            album: format!("{artist} album"),
            genre: genre.into(),
            year: Some(year),
            ..Default::default()
        };
        musicwasp_core::tags::write(&path, &edit).unwrap();
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    Album { lib, _dir: dir }
}

fn titles(tracks: &[musicwasp_core::Track]) -> Vec<String> {
    tracks.iter().map(|t| t.title.clone()).collect()
}

#[test]
fn smart_playlist_filters_with_and_any_and_sorts() {
    let album = library_with_tracks();

    // Rock tracks only.
    let rock = Definition {
        rules: vec![Rule {
            field: Field::Genre,
            op: Op::Equals,
            value: "Rock".into(),
        }],
        ..Default::default()
    };
    assert_eq!(
        titles(&album.lib.evaluate_smart(&rock).unwrap()),
        ["First", "Third"]
    );

    // Numeric rule.
    let recent = Definition {
        rules: vec![Rule {
            field: Field::Year,
            op: Op::Greater,
            value: "2000".into(),
        }],
        ..Default::default()
    };
    // Default sort is artist-then-album, so Alpha's track precedes Beta's.
    assert_eq!(
        titles(&album.lib.evaluate_smart(&recent).unwrap()),
        ["Third", "Second"]
    );

    // match_all = false behaves like OR.
    let any = Definition {
        match_all: false,
        rules: vec![
            Rule {
                field: Field::Genre,
                op: Op::Equals,
                value: "Jazz".into(),
            },
            Rule {
                field: Field::Genre,
                op: Op::Equals,
                value: "Ambient".into(),
            },
        ],
        ..Default::default()
    };
    assert_eq!(
        titles(&album.lib.evaluate_smart(&any).unwrap()),
        ["Second", "Fourth"]
    );

    // Sorting + limit.
    let newest = Definition {
        sort: SortBy::YearNewest,
        limit: Some(2),
        ..Default::default()
    };
    assert_eq!(
        titles(&album.lib.evaluate_smart(&newest).unwrap()),
        ["Third", "Second"]
    );

    let shortest = Definition {
        sort: SortBy::ShortestFirst,
        ..Default::default()
    };
    assert_eq!(
        titles(&album.lib.evaluate_smart(&shortest).unwrap()),
        ["First", "Third", "Second", "Fourth"]
    );

    // Empty rules means the whole library.
    assert_eq!(
        album
            .lib
            .evaluate_smart(&Definition::default())
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn smart_playlist_crud_and_persistence() {
    let album = library_with_tracks();

    let definition = Definition {
        rules: vec![Rule {
            field: Field::Artist,
            op: Op::Contains,
            value: "alph".into(), // case-insensitive LIKE
        }],
        sort: SortBy::Title,
        ..Default::default()
    };

    let id = album
        .lib
        .create_smart_playlist("Alpha only", &definition)
        .unwrap();

    let stored = album.lib.smart_playlists().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].name, "Alpha only");
    assert_eq!(stored[0].definition, definition);

    assert_eq!(
        titles(&album.lib.smart_playlist_tracks(id).unwrap()),
        ["First", "Third"]
    );

    // Editing updates the stored definition and the evaluated tracks.
    let edited = Definition {
        rules: vec![Rule {
            field: Field::Genre,
            op: Op::Equals,
            value: "Jazz".into(),
        }],
        ..Default::default()
    };
    album.lib.update_smart_playlist(id, &edited).unwrap();
    assert_eq!(
        titles(&album.lib.smart_playlist_tracks(id).unwrap()),
        ["Second"]
    );
    assert_eq!(album.lib.smart_playlists().unwrap()[0].definition, edited);

    // A smart playlist never stores rows, so deleting it is cheap.
    album.lib.delete_smart_playlist(id).unwrap();
    assert!(album.lib.smart_playlists().unwrap().is_empty());
    assert!(album.lib.smart_playlist_tracks(id).unwrap().is_empty());
}

#[test]
fn corrupt_definitions_fall_back_to_defaults() {
    let album = library_with_tracks();
    let conn = musicwasp_core::db::open_at(&album.lib.db_path().unwrap()).unwrap();
    conn.execute(
        "INSERT INTO smart_playlists (name, definition) VALUES ('broken', 'not json')",
        [],
    )
    .unwrap();

    let stored = smart::list(&conn).unwrap();
    let broken = stored.iter().find(|p| p.name == "broken").unwrap();
    assert_eq!(broken.definition, Definition::default());
    // And evaluating it does not error out.
    assert_eq!(smart::tracks(&conn, broken.id).unwrap().len(), 4);
    assert_eq!(
        album
            .lib
            .query(&Filter::default(), SortKey::Title, true)
            .unwrap()
            .len(),
        4
    );
}
