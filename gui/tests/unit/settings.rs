use super::*;

#[test]
fn tokens_round_trip() {
    for token in [
        "space",
        "enter",
        "f2",
        "ctrl+right",
        "ctrl+shift+space",
        "ctrl+shift+right",
        "ctrl+shift+left",
        "alt+period",
    ] {
        let parsed = Shortcut::parse(token).expect(token);
        assert_eq!(parsed.token(), token);
    }
}

#[test]
fn unknown_tokens_are_rejected() {
    assert!(Shortcut::parse("ctrl+nosuchkey").is_none());
    assert!(Shortcut::parse("").is_none());
}

#[test]
fn defaults_are_complete_and_conflict_free() {
    let settings = Settings::default();
    for action in Action::ALL {
        assert!(settings.shortcut(action).is_some(), "{action:?}");
    }
    assert!(settings.conflicts().is_empty());
}

#[test]
fn unbinding_disables_the_action() {
    let mut settings = Settings::default();
    settings.unbind(Action::PlayPause);
    assert!(settings.shortcut(Action::PlayPause).is_none());
    assert!(settings.conflicts().is_empty());

    // Survives a save/load cycle instead of silently reverting.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    settings.save(&path).unwrap();
    assert!(Settings::load(&path).shortcut(Action::PlayPause).is_none());
}

#[test]
fn rebinding_detects_conflicts() {
    let mut settings = Settings::default();
    let play = settings.shortcut(Action::PlayPause).unwrap();
    settings.set(Action::Stop, play);
    let conflicts = settings.conflicts();
    assert!(conflicts.contains(&Action::PlayPause));
    assert!(conflicts.contains(&Action::Stop));
}

#[test]
fn column_config_round_trips_and_normalizes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    let mut settings = Settings::default();
    settings.columns.layout = TrackLayout::AlbumArt;
    settings.columns.order = vec![
        TrackColumn::Album,
        TrackColumn::Title,
        TrackColumn::Artist,
        TrackColumn::Album,
    ];
    settings.columns.hidden = vec![TrackColumn::Genre, TrackColumn::Format];
    settings.columns.set_width(TrackColumn::Artist, 222.0);
    settings.columns.set_width(TrackColumn::Year, 1.0);
    settings.save(&path).unwrap();

    let loaded = Settings::load(&path);
    assert_eq!(loaded.columns.layout, TrackLayout::AlbumArt);
    assert_eq!(loaded.columns.width(TrackColumn::Artist), 222.0);
    assert_eq!(
        loaded.columns.width(TrackColumn::Year),
        TrackColumn::Year.min_width(),
        "widths are clamped to a usable minimum"
    );
    assert_eq!(
        loaded.columns.normalized_order().len(),
        TrackColumn::ALL.len(),
        "duplicates and missing columns are repaired"
    );
    assert_eq!(loaded.columns.normalized_order()[0], TrackColumn::Album);
    assert_eq!(
        loaded.columns.visible(),
        vec![
            TrackColumn::Album,
            TrackColumn::Title,
            TrackColumn::Artist,
            TrackColumn::Number,
            TrackColumn::Year,
            TrackColumn::Duration,
            TrackColumn::Rating,
        ]
    );
}

#[test]
fn hiding_every_column_still_leaves_a_usable_table() {
    let mut columns = ColumnConfig::default();
    columns.hidden = TrackColumn::ALL.to_vec();
    assert_eq!(columns.visible(), vec![TrackColumn::Title]);

    columns.move_column(TrackColumn::Format, -1);
    let order = columns.normalized_order();
    let format = order
        .iter()
        .position(|column| *column == TrackColumn::Format);
    let rating = order
        .iter()
        .position(|column| *column == TrackColumn::Rating);
    assert_eq!(rating, format.map(|index| index + 1));

    columns.reset();
    assert_eq!(columns, ColumnConfig::default());
}

#[test]
fn saved_settings_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    let mut settings = Settings::default();
    settings.set(Action::NextTrack, Shortcut::parse("alt+n").unwrap());
    settings.save(&path).unwrap();

    let loaded = Settings::load(&path);
    assert_eq!(loaded.shortcut(Action::NextTrack).unwrap().token(), "alt+n");
    assert_eq!(
        loaded.shortcut(Action::PlayPause).unwrap().token(),
        "space",
        "unlisted actions keep their defaults"
    );

    // Corrupt file must not take the app down.
    std::fs::write(&path, "{ not json").unwrap();
    let fallback = Settings::load(&path);
    assert_eq!(
        fallback.shortcut(Action::PlayPause).unwrap().token(),
        "space"
    );
}

#[test]
fn equalizer_and_skin_survive_a_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    let mut settings = Settings::default();
    settings.equalizer.enabled = true;
    settings.equalizer.gains_db[2] = 4.5;
    settings.equalizer.preamp_db = -2.0;
    settings.skin = crate::theme::Skin::MIDNIGHT;
    settings.save(&path).unwrap();

    let loaded = Settings::load(&path);
    assert!(loaded.equalizer.enabled);
    assert_eq!(loaded.equalizer.gains_db[2], 4.5);
    assert_eq!(loaded.equalizer.preamp_db, -2.0);
    assert_eq!(loaded.skin, crate::theme::Skin::MIDNIGHT);
}

#[test]
fn folded_browser_sections_survive_a_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    let mut settings = Settings::default();
    settings.folded.genre = true;
    settings.folded.playlists = true;
    settings.queue_open = false;
    settings.save(&path).unwrap();

    let loaded = Settings::load(&path);
    assert!(loaded.folded.genre);
    assert!(!loaded.folded.artist);
    assert!(loaded.folded.playlists);
    assert!(!loaded.queue_open);

    // Files written before the feature existed start fully expanded, with
    // the queue panel visible.
    std::fs::write(&path, r#"{"shortcuts":{"play_pause":"space"}}"#).unwrap();
    let old = Settings::load(&path);
    assert_eq!(old.folded, FoldedSections::default());
    assert!(old.queue_open);
}

#[test]
fn session_survives_a_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    let settings = Settings {
        watch_enabled: false,
        large_fonts: true,
        language: Language::French,
        replaygain: false,
        replaygain_album: true,
        output_device: Some("Speakers".to_string()),
        crossfade_secs: 4.5,
        scrobble_listenbrainz: true,
        listenbrainz_token: "token".to_string(),
        watch_folders: vec!["/music".to_string()],
        session: Session {
            view: SessionView::Playlist(7),
            search: "radio".to_string(),
            selected: vec![4, 2],
            volume: Some(0.42),
            queue: vec![3, 1, 2],
            queue_pos: Some(1),
            window: Some(WindowGeometry {
                width: 1100.0,
                height: 720.0,
                x: Some(40.0),
                y: Some(25.0),
            }),
        },
        ..Settings::default()
    };
    settings.save(&path).unwrap();

    let loaded = Settings::load(&path);
    assert_eq!(loaded.session, settings.session);
    assert!(!loaded.watch_enabled);
    assert!(loaded.large_fonts);
    assert_eq!(loaded.language, Language::French);
    assert_eq!(loaded.watch_folders, vec!["/music".to_string()]);
    assert!(!loaded.replaygain);
    assert!(loaded.replaygain_album);
    assert_eq!(loaded.output_device.as_deref(), Some("Speakers"));
    assert_eq!(loaded.crossfade_secs, 4.5);
    assert!(loaded.scrobble_listenbrainz);
    assert_eq!(loaded.listenbrainz_token, "token");
}

#[test]
fn f2_moves_from_the_tag_editor_to_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"shortcuts":{"play_pause":"space","edit_tags":"f2"}}"#,
    )
    .unwrap();

    let loaded = Settings::load(&path);
    assert_eq!(loaded.shortcut(Action::Rename).unwrap().token(), "f2");
    assert_eq!(
        loaded.shortcut(Action::EditTags).unwrap().token(),
        "ctrl+f2"
    );
    assert!(loaded.conflicts().is_empty());
}

#[test]
fn old_settings_without_new_fields_keep_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, r#"{"shortcuts":{"play_pause":"space"}}"#).unwrap();

    let loaded = Settings::load(&path);
    assert!(!loaded.equalizer.enabled);
    assert_eq!(loaded.skin, crate::theme::Skin::default());
    assert!(!loaded.large_fonts);
    assert_eq!(loaded.language, Language::English);
    assert_eq!(loaded.shortcut(Action::PlayPause).unwrap().token(), "space");
    assert_eq!(loaded.session, Session::default());
    assert!(loaded.watch_enabled);
    assert!(loaded.watch_folders.is_empty());
    assert!(loaded.replaygain);
    assert!(!loaded.replaygain_album);
    assert!(loaded.output_device.is_none());
    assert_eq!(loaded.crossfade_secs, 0.0);
    assert!(!loaded.scrobble_listenbrainz);
    assert!(loaded.listenbrainz_token.is_empty());
}
