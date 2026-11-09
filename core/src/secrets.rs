//! Secrets kept out of `settings.json`, and private file writes.
//!
//! The ListenBrainz token lives in the operating system's credential store:
//! Secret Service on Linux, Keychain on macOS and Credential Manager on Windows.

use anyhow::{Context, Result};
use keyring::{Entry, Error};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

const SERVICE: &str = "MusicWasp";
const LISTENBRAINZ_USER: &str = "listenbrainz-token";

/// Token last read from or written to the store, `None` when unknown. Saves
/// compare against it so routine settings writes do not hit the credential
/// store on every change.
static LAST_SYNCED: Mutex<Option<String>> = Mutex::new(None);

fn synced() -> std::sync::MutexGuard<'static, Option<String>> {
    LAST_SYNCED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn listenbrainz_entry() -> Result<Entry> {
    Entry::new(SERVICE, LISTENBRAINZ_USER).context("cannot open the OS credential store")
}

/// The stored ListenBrainz token, or `None` when nothing is stored.
pub fn listenbrainz_token() -> Result<Option<String>> {
    let token = match listenbrainz_entry()?.get_password() {
        Ok(token) => Some(token),
        Err(Error::NoEntry) => None,
        Err(error) => return Err(error).context("cannot read the ListenBrainz token"),
    };
    *synced() = Some(token.clone().unwrap_or_default());
    Ok(token)
}

/// Store a non-empty token. Writes are skipped when the value is unchanged.
pub fn set_listenbrainz_token(token: &str) -> Result<()> {
    anyhow::ensure!(!token.is_empty(), "refusing to store an empty token");
    let mut last = synced();
    if last.as_deref() == Some(token) {
        return Ok(());
    }
    listenbrainz_entry()?
        .set_password(token)
        .context("cannot save the ListenBrainz token")?;
    *last = Some(token.to_string());
    Ok(())
}

/// Remove the stored token. A missing entry is not an error.
pub fn delete_listenbrainz_token() -> Result<()> {
    let mut last = synced();
    match listenbrainz_entry()?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => {}
        Err(error) => return Err(error).context("cannot remove the ListenBrainz token"),
    }
    *last = Some(String::new());
    Ok(())
}

/// Write `data` to `path`, readable and writable only by the owner (0600 on
/// Unix). An existing file or symlink at `path` is replaced, never followed.
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_replace_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        std::fs::write(&path, "old").unwrap();
        write_private(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        write_private(&path, b"{}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_replaced_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "keep").unwrap();
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_private(&link, b"secret").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }
}
