//! The cache: response bodies, prices, and where cover files ended up.
//!
//! Three tables, three lifetimes. Metadata is stable and cached for thirty days;
//! a price is a fact about today and is cached for one; cover art is a file on
//! disk with only its path in the database, because a database full of JPEGs is
//! a database you cannot copy or inspect.
//!
//! **Misses are cached too.** An album with no art anywhere would otherwise run
//! the whole fallback chain on every single search, and the Dynamic Range
//! Database — which rate-limits hard — would be asked again for every album it
//! has never heard of. A negative entry expires sooner than a positive one,
//! because "not there yet" turns into "there now" more often than the reverse.
//!
//! Everything here is deterministic and local, so the tests run it against a
//! real in-memory database rather than a fake.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

/// Thirty days. Release metadata does not move.
pub const METADATA_TTL: i64 = 30 * 24 * 60 * 60;
/// One day. A price is only true today.
pub const PRICE_TTL: i64 = 24 * 60 * 60;
/// Thirty days for a cover that exists.
pub const ART_TTL: i64 = 30 * 24 * 60 * 60;
/// Seven days for one that does not, anywhere.
pub const ART_MISS_TTL: i64 = 7 * 24 * 60 * 60;

/// What kind of thing is being cached, and therefore how long it lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Metadata,
    Price,
}

impl Kind {
    pub fn ttl(self) -> i64 {
        match self {
            Kind::Metadata => METADATA_TTL,
            Kind::Price => PRICE_TTL,
        }
    }
}

/// What went wrong talking to the cache.
///
/// Every call site treats a failure as a miss and carries on: a cache that
/// cannot be read is a slow application, not a broken one, and there is nothing
/// a person could do about it anyway.
#[derive(Debug)]
pub struct CacheError(pub String);

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<rusqlite::Error> for CacheError {
    fn from(error: rusqlite::Error) -> Self {
        CacheError(error.to_string())
    }
}

type Result<T> = std::result::Result<T, CacheError>;

pub struct Cache {
    connection: Connection,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Cache> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Cache::from_connection(Connection::open(path)?)
    }

    /// An in-memory cache, for tests.
    pub fn in_memory() -> Result<Cache> {
        Cache::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Cache> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS responses (
                 key      TEXT PRIMARY KEY,
                 body     BLOB NOT NULL,
                 stored   INTEGER NOT NULL,
                 expires  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS covers (
                 key      TEXT PRIMARY KEY,
                 path     TEXT,
                 expires  INTEGER NOT NULL
             );",
        )?;
        Ok(Cache { connection })
    }

    // -- response bodies ----------------------------------------------------

    /// A cached body, if one is there and still current.
    pub fn body(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let body = self
            .connection
            .query_row(
                "SELECT body FROM responses WHERE key = ?1 AND expires > ?2",
                params![key, now()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        Ok(body)
    }

    pub fn store_body(&self, key: &str, kind: Kind, body: &[u8]) -> Result<()> {
        let now = now();
        self.connection.execute(
            "INSERT INTO responses (key, body, stored, expires) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET body = ?2, stored = ?3, expires = ?4",
            params![key, body, now, now + kind.ttl()],
        )?;
        Ok(())
    }

    // -- cover art ----------------------------------------------------------

    /// Where a cover was saved, or that there is not one.
    ///
    /// Three states, and the middle one is the point: `None` means "never
    /// looked", `Some(None)` means "looked, and there is no art anywhere".
    #[allow(clippy::option_option)]
    pub fn cover(&self, key: &str) -> Result<Option<Option<PathBuf>>> {
        let row = self
            .connection
            .query_row(
                "SELECT path FROM covers WHERE key = ?1 AND expires > ?2",
                params![key, now()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(row.map(|path| path.map(PathBuf::from)))
    }

    pub fn store_cover(&self, key: &str, path: &Path) -> Result<()> {
        self.connection.execute(
            "INSERT INTO covers (key, path, expires) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET path = ?2, expires = ?3",
            params![key, path.to_string_lossy(), now() + ART_TTL],
        )?;
        Ok(())
    }

    /// Record that nothing in the fallback chain had a cover for this.
    pub fn store_cover_miss(&self, key: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO covers (key, path, expires) VALUES (?1, NULL, ?2)
             ON CONFLICT(key) DO UPDATE SET path = NULL, expires = ?2",
            params![key, now() + ART_MISS_TTL],
        )?;
        Ok(())
    }

    // -- housekeeping -------------------------------------------------------

    /// Drop everything that has expired. Returns how many rows went.
    ///
    /// Cover files whose rows expire are left on disk for [`Cache::sweep_files`]
    /// to find, so that deleting a row can never delete a file another row still
    /// points at.
    pub fn purge_expired(&self) -> Result<usize> {
        let now = now();
        let responses = self
            .connection
            .execute("DELETE FROM responses WHERE expires <= ?1", params![now])?;
        let covers = self
            .connection
            .execute("DELETE FROM covers WHERE expires <= ?1", params![now])?;
        Ok(responses + covers)
    }

    /// Delete cover files no live row points at.
    pub fn sweep_files(&self, directory: &Path) -> Result<usize> {
        let mut statement = self
            .connection
            .prepare("SELECT path FROM covers WHERE path IS NOT NULL")?;
        let live: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(std::result::Result::ok)
            .collect();

        let Ok(entries) = std::fs::read_dir(directory) else {
            return Ok(0);
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if !live.iter().any(|kept| Path::new(kept) == path)
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Throw everything away.
    pub fn clear(&self) -> Result<()> {
        self.connection
            .execute_batch("DELETE FROM responses; DELETE FROM covers;")?;
        Ok(())
    }
}

/// A stable filename for a cache key.
///
/// The key cannot be the filename — it is a URL, so it contains slashes always
/// and exceeds 255 bytes often. This is a cache key and not a security boundary,
/// so FNV-1a is the right size of tool, and eight lines beats a dependency.
pub fn file_name_for(key: &str, extension: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.{extension}")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_body_comes_back() {
        let cache = Cache::in_memory().unwrap();
        cache
            .store_body("https://example/x", Kind::Metadata, b"hello")
            .unwrap();
        assert_eq!(
            cache.body("https://example/x").unwrap(),
            Some(b"hello".to_vec())
        );
        assert_eq!(cache.body("https://example/y").unwrap(), None);
    }

    #[test]
    fn storing_the_same_key_twice_replaces_rather_than_failing() {
        let cache = Cache::in_memory().unwrap();
        cache.store_body("k", Kind::Price, b"old").unwrap();
        cache.store_body("k", Kind::Price, b"new").unwrap();
        assert_eq!(cache.body("k").unwrap(), Some(b"new".to_vec()));
    }

    #[test]
    fn prices_expire_a_month_before_metadata_does() {
        // The two lifetimes the brief asks for, asserted as a relationship
        // rather than as two numbers that could drift apart.
        assert_eq!(Kind::Price.ttl(), 24 * 60 * 60);
        assert_eq!(Kind::Metadata.ttl(), 30 * 24 * 60 * 60);
        assert!(Kind::Price.ttl() < Kind::Metadata.ttl());
    }

    #[test]
    fn an_expired_entry_reads_as_absent_and_purges() {
        let cache = Cache::in_memory().unwrap();
        cache
            .connection
            .execute(
                "INSERT INTO responses (key, body, stored, expires) VALUES ('k', X'00', 0, 1)",
                [],
            )
            .unwrap();
        assert_eq!(cache.body("k").unwrap(), None);
        assert_eq!(cache.purge_expired().unwrap(), 1);
    }

    #[test]
    fn a_cover_miss_is_remembered_and_is_not_the_same_as_never_looking() {
        // The distinction that stops an album with no art re-running the whole
        // fallback chain on every keystroke.
        let cache = Cache::in_memory().unwrap();
        assert_eq!(cache.cover("mbid-1").unwrap(), None, "never looked");

        cache.store_cover_miss("mbid-1").unwrap();
        assert_eq!(
            cache.cover("mbid-1").unwrap(),
            Some(None),
            "looked, nothing there"
        );

        cache
            .store_cover("mbid-1", Path::new("/covers/a.jpg"))
            .unwrap();
        assert_eq!(
            cache.cover("mbid-1").unwrap(),
            Some(Some(PathBuf::from("/covers/a.jpg")))
        );
    }

    #[test]
    fn a_missing_cover_is_retried_sooner_than_a_found_one_is_refreshed() {
        // "Not there yet" turns into "there now" far more often than the
        // reverse, so a negative entry has to be the shorter-lived of the two.
        const _: () = assert!(ART_MISS_TTL < ART_TTL);
    }

    #[test]
    fn cache_filenames_are_stable_distinct_and_actually_filenames() {
        let a = file_name_for("https://coverartarchive.org/release/aaa/front-500", "jpg");
        let b = file_name_for("https://coverartarchive.org/release/bbb/front-500", "jpg");
        assert_eq!(
            a,
            file_name_for("https://coverartarchive.org/release/aaa/front-500", "jpg")
        );
        assert_ne!(a, b);
        assert!(!a.contains('/'));
        assert!(a.ends_with(".jpg"));
    }

    #[test]
    fn sweeping_removes_orphaned_files_and_keeps_referenced_ones() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("kept.jpg");
        let orphan = dir.path().join("orphan.jpg");
        std::fs::write(&kept, b"a").unwrap();
        std::fs::write(&orphan, b"b").unwrap();

        let cache = Cache::in_memory().unwrap();
        cache.store_cover("k", &kept).unwrap();

        assert_eq!(cache.sweep_files(dir.path()).unwrap(), 1);
        assert!(kept.exists());
        assert!(!orphan.exists());
    }

    #[test]
    fn a_cache_file_survives_being_reopened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cache.sqlite");
        {
            let cache = Cache::open(&path).unwrap();
            cache.store_body("k", Kind::Metadata, b"v").unwrap();
        }
        let reopened = Cache::open(&path).unwrap();
        assert_eq!(reopened.body("k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn clearing_empties_both_tables() {
        let cache = Cache::in_memory().unwrap();
        cache.store_body("k", Kind::Metadata, b"v").unwrap();
        cache.store_cover("c", Path::new("/x.jpg")).unwrap();
        cache.clear().unwrap();
        assert_eq!(cache.body("k").unwrap(), None);
        assert_eq!(cache.cover("c").unwrap(), None);
    }
}
