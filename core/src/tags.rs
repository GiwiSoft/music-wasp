//! Writing tags back to files.

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use lofty::tag::{Tag, TagExt};
use std::path::Path;

use crate::library::Track;

/// Editable tag fields. Empty strings clear the frame.
#[derive(Debug, Clone, Default)]
pub struct TagEdit {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<i32>,
    pub track_no: Option<i32>,
    pub disc_no: Option<i32>,
}

impl TagEdit {
    /// True when nothing would be written.
    pub fn is_empty(&self) -> bool {
        self.title.trim().is_empty()
            && self.artist.trim().is_empty()
            && self.album.trim().is_empty()
            && self.album_artist.trim().is_empty()
            && self.genre.trim().is_empty()
            && self.year.is_none()
            && self.track_no.is_none()
            && self.disc_no.is_none()
    }

    pub fn from_track(t: &Track) -> Self {
        Self {
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            genre: t.genre.clone(),
            year: t.year,
            track_no: t.track_no,
            disc_no: t.disc_no,
        }
    }
}

/// Write `edit` into the file's primary tag, clearing fields left empty.
pub fn write(path: &Path, edit: &TagEdit) -> Result<()> {
    write_fields(path, edit, false)
}

/// Write `edit` into the file's primary tag.
///
/// With `skip_empty`, empty fields are left untouched, which is what batch
/// editing needs: you only overwrite the fields you actually filled in.
pub fn write_fields(path: &Path, edit: &TagEdit, skip_empty: bool) -> Result<()> {
    let mut tagged = Probe::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .read()?;

    if tagged.primary_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }

    let tag = tagged
        .primary_tag_mut()
        .context("file has no writable tag")?;

    let set_or_clear = |tag: &mut Tag, value: &str, key: ItemKey| {
        if value.trim().is_empty() {
            if !skip_empty {
                tag.remove_key(&key);
            }
        } else {
            tag.insert_text(key, value.trim().to_string());
        }
    };

    set_or_clear(tag, &edit.title, ItemKey::TrackTitle);
    set_or_clear(tag, &edit.artist, ItemKey::TrackArtist);
    set_or_clear(tag, &edit.album, ItemKey::AlbumTitle);
    set_or_clear(tag, &edit.album_artist, ItemKey::AlbumArtist);
    set_or_clear(tag, &edit.genre, ItemKey::Genre);

    if let Some(year) = edit.year {
        tag.set_year(year as u32);
    } else if !skip_empty {
        tag.remove_year();
    }
    if let Some(track_no) = edit.track_no {
        tag.set_track(track_no as u32);
    } else if !skip_empty {
        tag.remove_track();
    }
    if let Some(disc_no) = edit.disc_no {
        tag.set_disk(disc_no as u32);
    } else if !skip_empty {
        tag.remove_disk();
    }

    tag.save_to_path(path, WriteOptions::default())
        .with_context(|| format!("cannot write tags to {}", path.display()))?;
    Ok(())
}
