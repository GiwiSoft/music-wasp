use anyhow::Context;
use clap::{Parser, Subcommand};
use musicwasp_core::{db, library, m3u, playlist, Player};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "musicwasp",
    version,
    about = "MusicWasp - a native music manager and player"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a directory and add tracks to the library
    Scan {
        path: PathBuf,
        /// Re-read tags even if files are unchanged
        #[arg(long)]
        rescan: bool,
    },
    /// List tracks in the library
    List {
        /// Substring filter on title/artist/album/genre
        #[arg(short, long)]
        filter: Option<String>,
    },
    /// Play a library track id, or a file path
    Play {
        /// Library track id or file path
        target: String,
        /// Stop after N seconds (0 = whole track)
        #[arg(long, default_value_t = 0)]
        seconds: u64,
    },
    /// Look an album up on MusicBrainz (optionally tag tracks with it)
    Autotag {
        #[arg(short = 'r', long)]
        artist: String,
        #[arg(short = 'l', long)]
        album: String,
        /// Release id to apply; "best" picks the top candidate
        #[arg(long)]
        apply: Option<String>,
        /// Library track ids to tag, comma separated
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
        /// Also download and embed the front cover
        #[arg(long)]
        cover: bool,
    },
    /// Show library statistics
    Stats,
    /// Manage playlists, folders and M3U files
    Playlist {
        #[command(subcommand)]
        command: PlaylistCommand,
    },
}

#[derive(Subcommand)]
enum PlaylistCommand {
    /// List playlists and their folders
    List,
    /// Create a playlist from comma-separated track ids
    Create {
        name: String,
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
        /// Folder id or name
        #[arg(long)]
        folder: Option<String>,
    },
    /// Import an M3U/M3U8 file (audio files are indexed automatically)
    Import {
        file: PathBuf,
        /// Override the playlist name (defaults to the file name)
        #[arg(long)]
        name: Option<String>,
        /// Folder id or name
        #[arg(long)]
        folder: Option<String>,
    },
    /// Export a playlist by id or name
    Export {
        /// Playlist id or exact name
        playlist: String,
        output: PathBuf,
    },
    /// Create a playlist folder
    FolderCreate {
        name: String,
        /// Parent folder id or name
        #[arg(long)]
        parent: Option<String>,
    },
    /// List playlist folders
    Folders,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let cli = Cli::parse();
    let conn = db::open()?;

    match cli.command {
        Commands::Scan { path, rescan } => {
            let report = library::scan_with_progress(&conn, &path, rescan, |_| true)?;
            println!(
                "{} tracks: {} new/updated, {} unchanged, {} failed",
                report.found, report.indexed, report.unchanged, report.failed
            );
        }
        Commands::List { filter } => {
            let tracks = library::list(&conn, filter.as_deref())?;
            if tracks.is_empty() {
                println!("library empty - run: musicwasp-cli scan <dir>");
                return Ok(());
            }
            for t in &tracks {
                println!(
                    "{:>4}  {:<28} {:<24} {:<28} {:>5}  {}",
                    t.id,
                    trunc(&t.artist, 28),
                    trunc(&t.album, 24),
                    trunc(&t.title, 28),
                    fmt_time(t.duration_secs),
                    t.format
                );
            }
            println!("\n{} tracks", tracks.len());
        }
        Commands::Play { target, seconds } => {
            let path = match target.parse::<i64>() {
                Ok(id) => library::path_for_id(&conn, id)?
                    .ok_or_else(|| anyhow::anyhow!("no track with id {id}"))?,
                Err(_) => PathBuf::from(target),
            };
            let player = Player::new()?;
            player.load_and_play(&path);
            println!("playing {}", path.display());

            let deadline = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if player.take_finished() {
                    break;
                }
                if seconds > 0 && deadline.elapsed().as_secs() >= seconds {
                    break;
                }
                if player.snapshot().status == musicwasp_core::PlayerStatus::Stopped {
                    break;
                }
            }
        }
        Commands::Autotag {
            artist,
            album,
            apply,
            ids,
            cover,
        } => {
            let candidates = musicwasp_core::autotag::search(&artist, &album)?;
            if candidates.is_empty() {
                println!("no releases found for {artist} — {album}");
                return Ok(());
            }

            println!(
                "{:>3}  {:<34} {:<24} {:>5} {:<6} id",
                "scr", "release", "artist", "trks", "date"
            );
            for c in &candidates {
                println!(
                    "{:>3}  {:<34} {:<24} {:>5} {:<6} {}",
                    c.score,
                    trunc(&c.title, 34),
                    trunc(&c.artist, 24),
                    c.track_count,
                    c.date.clone().unwrap_or_default(),
                    c.release_id
                );
            }

            let Some(target) = apply else {
                return Ok(());
            };

            let release_id = if target == "best" {
                candidates
                    .iter()
                    .max_by_key(|c| musicwasp_core::autotag::relevance(c, ids.len()))
                    .map(|c| c.release_id.clone())
                    .context("no MusicBrainz candidates to apply")?
            } else {
                target
            };

            if ids.is_empty() {
                anyhow::bail!("--apply needs --ids");
            }

            let tracks = library::tracks_by_ids(&conn, &ids)?;
            if tracks.len() != ids.len() {
                anyhow::bail!(
                    "{} of {} ids not found",
                    ids.len() - tracks.len(),
                    ids.len()
                );
            }

            let local: Vec<musicwasp_core::autotag::LocalTrack> = tracks
                .iter()
                .map(|t| musicwasp_core::autotag::LocalTrack {
                    path: PathBuf::from(&t.path),
                    disc_no: t.disc_no,
                    track_no: t.track_no,
                    title: t.title.clone(),
                    duration_secs: t.duration_secs,
                })
                .collect();

            let release = musicwasp_core::autotag::fetch(&release_id)?;
            let suggestions = musicwasp_core::autotag::suggest(&local, &release);
            let matches: Vec<musicwasp_core::autotag::Match> = suggestions
                .iter()
                .enumerate()
                .filter_map(|(index, choice)| {
                    let track = choice.and_then(|choice| release.tracks.get(choice))?;
                    Some(musicwasp_core::autotag::Match {
                        path: local[index].path.clone(),
                        track: track.clone(),
                    })
                })
                .collect();
            println!(
                "\n{}: {} of {} local tracks matched ({} on the release)",
                release.title,
                matches.len(),
                local.len(),
                release.tracks.len()
            );

            let cover_data = if cover {
                musicwasp_core::autotag::front_cover(&release_id)?
            } else {
                None
            };
            if cover_data.is_some() {
                println!("front cover downloaded");
            }

            let (written, errors) =
                musicwasp_core::autotag::apply(&release, &matches, cover_data.as_ref());
            for path in matches.iter().map(|m| &m.path) {
                library::refresh_file(&conn, path)?;
            }
            println!("tagged {written} file(s)");
            for error in errors {
                eprintln!("  {error}");
            }
        }
        Commands::Playlist { command } => match command {
            PlaylistCommand::List => {
                let folders = playlist::list_folders(&conn)?;
                if !folders.is_empty() {
                    println!("folders:");
                    for folder in &folders {
                        println!("  {:>4}  {}", folder.id, folder_label(&folders, folder.id));
                    }
                }
                let playlists = playlist::list(&conn)?;
                if playlists.is_empty() {
                    println!("no playlists");
                } else {
                    println!("{:>4}  {:<32} {:>5}  folder", "id", "name", "tracks");
                    for item in playlists {
                        let folder = item
                            .folder_id
                            .map(|id| folder_label(&folders, id))
                            .unwrap_or_default();
                        println!(
                            "{:>4}  {:<32} {:>5}  {}",
                            item.id,
                            trunc(&item.name, 32),
                            item.count,
                            folder
                        );
                    }
                }
            }
            PlaylistCommand::Create { name, ids, folder } => {
                let folder_id = resolve_folder(&conn, folder.as_deref())?;
                let id = playlist::create(&conn, &name, &ids)?;
                if folder_id.is_some() {
                    playlist::set_folder(&conn, id, folder_id)?;
                }
                println!("created playlist {id}: {name} ({} tracks)", ids.len());
            }
            PlaylistCommand::Import { file, name, folder } => {
                let folder_id = resolve_folder(&conn, folder.as_deref())?;
                let report = m3u::import(&conn, &file, name.as_deref(), folder_id)?;
                println!(
                    "imported \"{}\" (id {}): {} track(s)",
                    report.name, report.playlist_id, report.added
                );
                if report.missing > 0 {
                    println!("  {} path(s) missing", report.missing);
                }
                if report.unsupported > 0 {
                    println!("  {} path(s) not audio or unsupported", report.unsupported);
                }
            }
            PlaylistCommand::Export { playlist, output } => {
                let id = resolve_playlist(&conn, &playlist)?;
                let count = m3u::export(&conn, id, &output)?;
                println!("exported {count} track(s) to {}", output.display());
            }
            PlaylistCommand::FolderCreate { name, parent } => {
                let parent_id = resolve_folder(&conn, parent.as_deref())?;
                let id = playlist::create_folder(&conn, &name, parent_id)?;
                println!("created folder {id}: {name}");
            }
            PlaylistCommand::Folders => {
                let folders = playlist::list_folders(&conn)?;
                if folders.is_empty() {
                    println!("no playlist folders");
                }
                for folder in &folders {
                    println!("{:>4}  {}", folder.id, folder_label(&folders, folder.id));
                }
            }
        },
        Commands::Stats => {
            let s = library::stats(&conn)?;
            println!("tracks:   {}", s.tracks);
            println!("artists:  {}", s.artists);
            println!("albums:   {}", s.albums);
            println!("duration: {}", fmt_long(s.total_secs));
            println!("size:     {:.1} MB", s.total_bytes as f64 / 1_048_576.0);
            println!("db:       {}", db::path()?.display());
        }
    }

    Ok(())
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn resolve_playlist(conn: &rusqlite::Connection, value: &str) -> anyhow::Result<i64> {
    let items = playlist::list(conn)?;
    if let Ok(id) = value.parse::<i64>() {
        if items.iter().any(|item| item.id == id) {
            return Ok(id);
        }
    }
    items
        .into_iter()
        .find(|item| item.name.eq_ignore_ascii_case(value))
        .map(|item| item.id)
        .ok_or_else(|| anyhow::anyhow!("no playlist with id or name {value:?}"))
}

fn resolve_folder(conn: &rusqlite::Connection, value: Option<&str>) -> anyhow::Result<Option<i64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let folders = playlist::list_folders(conn)?;
    if let Ok(id) = value.parse::<i64>() {
        if folders.iter().any(|folder| folder.id == id) {
            return Ok(Some(id));
        }
    }
    folders
        .iter()
        .find(|folder| {
            folder.name.eq_ignore_ascii_case(value)
                || folder_label(&folders, folder.id).eq_ignore_ascii_case(value)
        })
        .map(|folder| Some(folder.id))
        .ok_or_else(|| anyhow::anyhow!("no playlist folder with id or name {value:?}"))
}

fn folder_label(folders: &[playlist::PlaylistFolder], folder_id: i64) -> String {
    let mut names = Vec::new();
    let mut current = Some(folder_id);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            break;
        }
        let Some(folder) = folders.iter().find(|folder| folder.id == id) else {
            break;
        };
        names.push(folder.name.clone());
        current = folder.parent_id;
    }
    names.reverse();
    names.join(" / ")
}

fn fmt_time(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn fmt_long(secs: u64) -> String {
    format!(
        "{}h {:02}m {:02}s",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}
