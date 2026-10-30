use musicwasp_core::{library, tags, Library, TagEdit};
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
fn album_groups_cover_embedding_and_sidecar_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("album");
    std::fs::create_dir_all(&music).unwrap();
    for (name, title) in [("one.wav", "One"), ("two.wav", "Two")] {
        let path = music.join(name);
        write_wav(&path, 4_410);
        tags::write(
            &path,
            &TagEdit {
                title: title.into(),
                artist: "Track Artist".into(),
                album: "Grouped Album".into(),
                album_artist: "Album Artist".into(),
                ..Default::default()
            },
        )
        .unwrap();
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    assert_eq!(lib.scan(&music, false).unwrap(), 2);
    let groups = lib.album_groups().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].album, "Grouped Album");
    assert_eq!(groups[0].artist, "Album Artist");
    assert_eq!(groups[0].track_count, 2);
    assert_eq!(
        lib.album_tracks("Grouped Album", "Album Artist")
            .unwrap()
            .len(),
        2
    );

    let first = music.join("one.wav");
    assert!(library::cover_art(&first).is_none());

    library::embed_cover(&first, b"front-cover", "image/jpeg").unwrap();
    assert_eq!(
        library::cover_art(&first).as_deref(),
        Some(&b"front-cover"[..])
    );

    let sidecar = library::save_cover_sidecar(&first, b"jpeg-sidecar").unwrap();
    assert!(sidecar.ends_with("cover.jpg"));
    assert_eq!(library::sidecar_cover(&first), Some(sidecar.clone()));
    assert_eq!(std::fs::read(sidecar).unwrap(), b"jpeg-sidecar");
}
