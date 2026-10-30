//! Writing tags back to files.

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use lofty::tag::{Tag, TagExt};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::library::Track;

/// Editable tag fields. Empty strings clear the frame.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// One embedded picture, stored in a form that survives JSON persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PictureSnapshot {
    pub pic_type: u8,
    pub mime: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub data: Vec<u8>,
}

/// Everything the app overwrites when it writes tags, captured before a write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSnapshot {
    pub path: PathBuf,
    pub edit: TagEdit,
    pub pictures: Vec<PictureSnapshot>,
}

impl TagSnapshot {
    /// Read the fields and pictures that tag writes can replace.
    pub fn capture(path: &Path) -> Result<Self> {
        let tagged = Probe::open(path)
            .with_context(|| format!("cannot open {}", path.display()))?
            .read()?;
        let Some(tag) = tagged.primary_tag() else {
            return Ok(Self {
                path: path.to_path_buf(),
                edit: TagEdit::default(),
                pictures: Vec::new(),
            });
        };

        let text = |key: ItemKey| tag.get_string(&key).unwrap_or_default().to_string();
        let edit = TagEdit {
            title: text(ItemKey::TrackTitle),
            artist: text(ItemKey::TrackArtist),
            album: text(ItemKey::AlbumTitle),
            album_artist: text(ItemKey::AlbumArtist),
            genre: text(ItemKey::Genre),
            year: tag.year().map(|year| year as i32),
            track_no: tag.track().map(|track| track as i32),
            disc_no: tag.disk().map(|disc| disc as i32),
        };
        let pictures = tag
            .pictures()
            .iter()
            .map(|picture| PictureSnapshot {
                pic_type: picture.pic_type().as_u8(),
                mime: picture.mime_type().map(|mime| mime.as_str().to_string()),
                description: picture.description().map(str::to_string),
                data: picture.data().to_vec(),
            })
            .collect();

        Ok(Self {
            path: path.to_path_buf(),
            edit,
            pictures,
        })
    }
}

/// Snapshot several files before a batch write.
pub fn snapshot_paths(paths: &[PathBuf]) -> Result<Vec<TagSnapshot>> {
    paths
        .iter()
        .map(|path| TagSnapshot::capture(path))
        .collect()
}

/// Restore a snapshot, including embedded pictures.
pub fn restore(snapshot: &TagSnapshot) -> Result<()> {
    write_fields(&snapshot.path, &snapshot.edit, false)?;

    let mut tagged = Probe::open(&snapshot.path)
        .with_context(|| format!("cannot open {}", snapshot.path.display()))?
        .read()?;
    if tagged.primary_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .context("file has no writable tag")?;

    while !tag.pictures().is_empty() {
        tag.remove_picture(0);
    }
    for picture in &snapshot.pictures {
        let mime = picture.mime.as_deref().map(MimeType::from_str);
        tag.push_picture(Picture::new_unchecked(
            PictureType::from_u8(picture.pic_type),
            mime,
            picture.description.clone(),
            picture.data.clone(),
        ));
    }

    tag.save_to_path(&snapshot.path, WriteOptions::default())
        .with_context(|| format!("cannot restore tags to {}", snapshot.path.display()))?;
    Ok(())
}

/// Persist the last undo batch atomically.
pub fn save_undo(path: &Path, snapshots: &[TagSnapshot]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec(snapshots)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Load the persisted undo batch, if one exists.
pub fn load_undo(path: &Path) -> Result<Option<Vec<TagSnapshot>>> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(serde_json::from_slice(&data)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Forget the undo batch.
pub fn clear_undo(path: &Path) {
    let _ = std::fs::remove_file(path);
}
