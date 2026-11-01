//! MusicWasp core: library database, metadata scanning, playback engine.

pub mod autotag;
pub mod db;
pub mod dsp;
pub mod library;
pub mod m3u;
pub mod player;
pub mod playlist;
pub mod scrobble;
pub mod smart;
pub mod tags;

pub use library::{AlbumGroup, Filter, Library, SortKey, Stats, Track};
pub use player::{Player, PlayerSnapshot, PlayerStatus};
pub use playlist::{Playlist, PlaylistFolder};
pub use smart::{Definition, Field, Op, Rule, SmartPlaylist, SortBy};
pub use tags::TagEdit;
