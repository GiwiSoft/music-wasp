use clap::{Parser, Subcommand};
use musicwasp_core::{db, library, Player};
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
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let cli = Cli::parse();
    let conn = db::open()?;

    match cli.command {
        Commands::Scan { path, rescan } => {
            let n = library::scan(&conn, &path, rescan)?;
            println!("{n} tracks indexed");
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
                    .unwrap()
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
