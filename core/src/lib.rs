//! MusicWasp core: library database, metadata scanning, playback engine.

pub mod analysis;
pub mod autotag;
pub mod backup;
pub mod bounded;
pub mod db;
pub mod dsp;
pub mod library;
pub mod lyrics;
pub mod m3u;
pub mod organize;
pub mod player;
pub mod playlist;
pub mod podcast;
pub mod radio;
pub mod reconcile;
pub mod scrobble;
pub mod search;
pub mod smart;
pub mod tags;

pub use library::{AlbumGroup, Filter, Library, ScanProgress, ScanReport, SortKey, Stats, Track};
pub use player::{Player, PlayerSnapshot, PlayerStatus};
pub use playlist::{Playlist, PlaylistFolder};
pub use reconcile::{ReconcileCandidate, ReconcileOutcome, ReconcilePlan};
pub use search::{Condition, NumberField, NumberOp, Query, SavedSearch, TextField};
pub use smart::{Definition, Field, Op, Rule, SmartPlaylist, SortBy};
pub use tags::TagEdit;
