use musicwasp_core::{search, Filter, Library, SortKey, TagEdit};
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

fn tag(
    path: &std::path::Path,
    title: &str,
    artist: &str,
    album: &str,
    genre: &str,
    year: i32,
    track: i32,
) {
    musicwasp_core::tags::write(
        path,
        &TagEdit {
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            genre: genre.to_string(),
            year: Some(year),
            track_no: Some(track),
            ..Default::default()
        },
    )
    .unwrap();
}

fn titles(lib: &Library, query: &str) -> Vec<String> {
    let tracks = lib
        .query(
            &Filter {
                text: query.to_string(),
                ..Default::default()
            },
            SortKey::Title,
            true,
        )
        .unwrap();
    tracks.into_iter().map(|track| track.title).collect()
}

#[test]
fn advanced_search_filters_fields_and_keeps_plain_text_search() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();

    let files = [
        (
            "airbag.wav",
            "Airbag",
            "Radiohead",
            "OK Computer",
            "Alternative",
            1997,
            1,
        ),
        (
            "karma.wav",
            "Karma Police",
            "Radiohead",
            "OK Computer",
            "Alternative",
            1997,
            6,
        ),
        (
            "creep.wav",
            "Creep",
            "Radiohead",
            "Pablo Honey",
            "Alternative",
            1993,
            1,
        ),
        (
            "dummy.wav",
            "Roads",
            "Portishead",
            "Dummy",
            "Trip Hop",
            1994,
            1,
        ),
    ];
    for (name, title, artist, album, genre, year, track) in files {
        let path = music.join(name);
        write_wav(&path, 500);
        tag(&path, title, artist, album, genre, year, track);
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();

    assert_eq!(
        titles(&lib, "artist:\"radiohead\" year:1997"),
        vec!["Airbag", "Karma Police"]
    );
    assert_eq!(titles(&lib, "artist:radiohead year:<1995"), vec!["Creep"]);
    assert_eq!(
        titles(&lib, "album:\"OK Computer\" track:1"),
        vec!["Airbag"]
    );
    assert_eq!(
        titles(&lib, "year:1990..1999 genre:alternative"),
        vec!["Airbag", "Creep", "Karma Police"]
    );
    assert_eq!(
        titles(&lib, "title:=Airbag"),
        vec!["Airbag"],
        "a leading equals means exact"
    );
    assert!(titles(&lib, "title:=Air").is_empty());
    assert_eq!(titles(&lib, "format:wav").len(), 4);
    assert_eq!(titles(&lib, "radiohead").len(), 3);
    assert!(titles(&lib, "unknown:value").is_empty());
}

#[test]
fn saved_searches_round_trip_and_validate_names() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();

    let id = lib
        .create_saved_search("Radiohead 1997", "artist:radiohead year:1997")
        .unwrap();
    let listed = lib.saved_searches().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Radiohead 1997");
    assert_eq!(listed[0].query, "artist:radiohead year:1997");

    lib.update_saved_search(id, "Radiohead", "artist:radiohead")
        .unwrap();
    let listed = lib.saved_searches().unwrap();
    assert_eq!(listed[0].name, "Radiohead");
    assert_eq!(listed[0].query, "artist:radiohead");

    assert!(lib.create_saved_search("  ", "year:1997").is_err());
    assert!(lib.create_saved_search("Empty", "  ").is_err());
    assert!(lib.update_saved_search(999, "Nope", "year:1997").is_err());

    lib.delete_saved_search(id).unwrap();
    assert!(lib.saved_searches().unwrap().is_empty());
    assert!(lib.delete_saved_search(id).is_err());
}

#[test]
fn parser_is_available_for_ui_or_tests() {
    let query = search::parse("artist:\"radiohead\" year:1997");
    assert!(query.text.is_empty());
    assert_eq!(query.conditions.len(), 2);
}
