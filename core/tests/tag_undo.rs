use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::Accessor;
use lofty::probe::Probe;
use lofty::tag::TagExt;
use musicwasp_core::{tags, TagEdit};
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

fn set_cover(path: &std::path::Path, data: &[u8]) {
    let mut tagged = Probe::open(path).unwrap().read().unwrap();
    let tag = tagged.primary_tag_mut().unwrap();
    while !tag.pictures().is_empty() {
        tag.remove_picture(0);
    }
    tag.push_picture(Picture::new_unchecked(
        PictureType::CoverFront,
        Some(MimeType::Png),
        None,
        data.to_vec(),
    ));
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

#[test]
fn undo_restores_metadata_and_cover_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("song.wav");
    write_wav(&path, 4_410);
    let undo_path = dir.path().join("tag-undo.json");

    tags::write(
        &path,
        &TagEdit {
            title: "Original".into(),
            artist: "Correct Artist".into(),
            album: "Correct Album".into(),
            album_artist: "Correct Artist".into(),
            genre: "Ambient".into(),
            year: Some(1997),
            track_no: Some(4),
            disc_no: Some(1),
            replay_gain: None,
        },
    )
    .unwrap();
    set_cover(&path, b"old-cover");

    let snapshots = tags::snapshot_paths(std::slice::from_ref(&path)).unwrap();
    tags::save_undo(&undo_path, &snapshots).unwrap();

    // Simulate a destructive auto-tag write.
    tags::write(
        &path,
        &TagEdit {
            title: "Wrong".into(),
            artist: "Wrong Artist".into(),
            album: "Wrong Album".into(),
            ..Default::default()
        },
    )
    .unwrap();
    set_cover(&path, b"new-cover");

    let loaded = tags::load_undo(&undo_path).unwrap().unwrap();
    tags::restore(&loaded[0]).unwrap();

    let tagged = Probe::open(&path).unwrap().read().unwrap();
    let tag = tagged.primary_tag().unwrap();
    assert_eq!(tag.title().as_deref(), Some("Original"));
    assert_eq!(tag.artist().as_deref(), Some("Correct Artist"));
    assert_eq!(tag.album().as_deref(), Some("Correct Album"));
    assert_eq!(tag.year(), Some(1997));
    assert_eq!(tag.track(), Some(4));
    assert_eq!(tag.disk(), Some(1));
    assert_eq!(tag.pictures()[0].data(), b"old-cover");

    tags::clear_undo(&undo_path);
    assert!(tags::load_undo(&undo_path).unwrap().is_none());
}

#[test]
fn replay_gain_round_trips_and_can_be_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gain.wav");
    write_wav(&path, 4_410);

    tags::write(
        &path,
        &TagEdit {
            title: "Gain".into(),
            replay_gain: Some(-6.5),
            ..Default::default()
        },
    )
    .unwrap();
    let read = tags::read(&path).unwrap();
    let gain = read.replay_gain.expect("replay gain was written");
    assert!((gain + 6.5).abs() < 0.01, "unexpected gain {gain}");

    // Editing another property must not drop the gain.
    let mut edit = read.clone();
    edit.title = "Renamed".into();
    tags::write(&path, &edit).unwrap();
    assert!(tags::read(&path).unwrap().replay_gain.is_some());

    tags::write(
        &path,
        &TagEdit {
            replay_gain: None,
            ..edit
        },
    )
    .unwrap();
    assert!(tags::read(&path).unwrap().replay_gain.is_none());
}
