use musicwasp_core::{playlist, Filter, Library, SortKey, TagEdit};
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
fn playlist_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();

    for (name, artist) in [("one.wav", "Artist A"), ("two.wav", "Artist B")] {
        let p = music.join(name);
        write_wav(&p, 800);
        let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
        let _ = lib; // first call migrates
        let edit = TagEdit {
            title: name.trim_end_matches(".wav").to_string(),
            artist: artist.to_string(),
            ..Default::default()
        };
        musicwasp_core::tags::write(&p, &edit).unwrap();
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    assert_eq!(lib.scan(&music, false).unwrap(), 2);
    let all = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    assert_eq!(all.len(), 2);

    let ids: Vec<i64> = all.iter().map(|t| t.id).collect();
    let pid = lib.create_playlist("Chill", &ids).unwrap();

    let stored = lib.playlist_tracks(pid).unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].title, "one", "playlist preserves insertion order");

    let listed = lib.playlists().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Chill");
    assert_eq!(listed[0].count, 2);

    // Adding a duplicate must not inflate the playlist.
    lib.add_to_playlist(pid, ids[0]).unwrap();
    assert_eq!(lib.playlist_tracks(pid).unwrap().len(), 2);

    // Rename and delete.
    let conn = musicwasp_core::db::open_at(&dir.path().join("library.db")).unwrap();
    playlist::rename(&conn, pid, "Focus").unwrap();
    assert_eq!(lib.playlists().unwrap()[0].name, "Focus");

    lib.delete_playlist(pid).unwrap();
    assert!(lib.playlists().unwrap().is_empty());
    assert!(lib.playlist_tracks(pid).unwrap().is_empty());
}

#[test]
fn batch_add_keeps_order_and_ignores_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    for name in ["a.wav", "b.wav", "c.wav"] {
        write_wav(&music.join(name), 400);
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    let ids: Vec<i64> = lib
        .query(&Filter::default(), SortKey::Title, true)
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids.len(), 3);

    let pid = lib.create_playlist("Mixed", &[ids[0]]).unwrap();
    let added = lib
        .add_many_to_playlist(pid, &[ids[1], ids[2], ids[1]])
        .unwrap();
    assert_eq!(added, 2, "the duplicate is skipped");

    let stored: Vec<i64> = lib
        .playlist_tracks(pid)
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(stored, ids, "order follows the drop order");
}

#[test]
fn reorder_and_remove_keep_positions_dense() {
    use musicwasp_core::playlist;

    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    for name in ["a.wav", "b.wav", "c.wav", "d.wav"] {
        write_wav(&music.join(name), 400);
    }

    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
    lib.scan(&music, false).unwrap();
    let all = lib.query(&Filter::default(), SortKey::Title, true).unwrap();
    let ids: Vec<i64> = all.iter().map(|t| t.id).collect();
    let pid = lib.create_playlist("Order", &ids).unwrap();

    let conn = test_conn(&dir);
    let order = || {
        let lib = Library::open_at(&dir.path().join("library.db")).unwrap();
        lib.playlist_tracks(pid)
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect::<Vec<i64>>()
    };

    // Move the last track to the front.
    playlist::move_tracks(&conn, pid, &[ids[3]], Some(ids[0])).unwrap();
    assert_eq!(order(), vec![ids[3], ids[0], ids[1], ids[2]]);

    // Move a block to the end.
    playlist::move_tracks(&conn, pid, &[ids[3], ids[0]], None).unwrap();
    assert_eq!(order(), vec![ids[1], ids[2], ids[3], ids[0]]);

    // Reordering is idempotent when nothing is asked to move.
    playlist::move_tracks(&conn, pid, &[], Some(ids[1])).unwrap();
    assert_eq!(order(), vec![ids[1], ids[2], ids[3], ids[0]]);

    // Remove from the middle.
    let removed = lib.remove_from_playlist(pid, &[ids[2]]).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(order(), vec![ids[1], ids[3], ids[0]]);

    // Removing something that is not there is a no-op.
    assert_eq!(lib.remove_from_playlist(pid, &[999_999]).unwrap(), 0);
    assert_eq!(order(), vec![ids[1], ids[3], ids[0]]);
}

#[test]
fn playlist_folders_group_nest_and_delete_without_losing_playlists() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open_at(&dir.path().join("library.db")).unwrap();

    let root = lib.create_playlist_folder("Rock", None).unwrap();
    let sub = lib.create_playlist_folder("Live", Some(root)).unwrap();
    let first = lib.create_playlist("First", &[]).unwrap();
    let second = lib.create_playlist("Second", &[]).unwrap();

    lib.move_playlist_to_folder(first, Some(sub)).unwrap();
    lib.move_playlist_to_folder(second, Some(root)).unwrap();

    let folders = lib.playlist_folders().unwrap();
    assert_eq!(folders.len(), 2);
    let live = folders.iter().find(|folder| folder.id == sub).unwrap();
    assert_eq!(live.name, "Live");
    assert_eq!(live.parent_id, Some(root));

    let playlists = lib.playlists().unwrap();
    assert_eq!(
        playlists
            .iter()
            .find(|playlist| playlist.id == first)
            .unwrap()
            .folder_id,
        Some(sub)
    );

    // Deleting a folder promotes its playlists and nested folders one level.
    lib.delete_playlist_folder(root).unwrap();
    let folders = lib.playlist_folders().unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].id, sub);
    assert_eq!(folders[0].parent_id, None);
    let playlists = lib.playlists().unwrap();
    assert_eq!(
        playlists
            .iter()
            .find(|playlist| playlist.id == first)
            .unwrap()
            .folder_id,
        Some(sub)
    );
    assert_eq!(
        playlists
            .iter()
            .find(|playlist| playlist.id == second)
            .unwrap()
            .folder_id,
        None
    );

    lib.rename_playlist_folder(sub, "Concerts").unwrap();
    assert_eq!(lib.playlist_folders().unwrap()[0].name, "Concerts");

    // Moving back to the root and deleting the last folder leaves playlists.
    lib.move_playlist_to_folder(first, None).unwrap();
    lib.delete_playlist_folder(sub).unwrap();
    assert!(lib.playlist_folders().unwrap().is_empty());
    assert_eq!(lib.playlists().unwrap().len(), 2);
    assert!(lib
        .playlists()
        .unwrap()
        .iter()
        .all(|playlist| playlist.folder_id.is_none()));
}

#[test]
fn playlist_folder_names_must_not_be_empty() {
    let dir = tempfile::tempdir().unwrap();
    let conn = test_conn(&dir);
    assert!(playlist::create_folder(&conn, "  ", None).is_err());
    assert!(playlist::rename_folder(&conn, 999, "Nope").is_err());
}

#[test]
fn old_databases_gain_the_playlist_folder_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE playlists (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             INSERT INTO playlists (name) VALUES ('Old');",
        )
        .unwrap();
    }

    let lib = Library::open_at(&path).unwrap();
    assert_eq!(lib.playlists().unwrap()[0].folder_id, None);
    let folder = lib.create_playlist_folder("New", None).unwrap();
    lib.move_playlist_to_folder(lib.playlists().unwrap()[0].id, Some(folder))
        .unwrap();
    assert_eq!(lib.playlists().unwrap()[0].folder_id, Some(folder));
}

fn test_conn(dir: &tempfile::TempDir) -> rusqlite::Connection {
    musicwasp_core::db::open_at(&dir.path().join("library.db")).unwrap()
}
