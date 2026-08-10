//! In-memory read-through cache for S3 record JSON, keyed by object key and
//! revalidated against the object's S3 ETag rather than a TTL.
//!
//! Every record-listing flow does a `ListObjectsV2` before its per-object
//! GETs, and that list carries each object's current ETag. So the list is the
//! freshness probe: if the cached entry's ETag matches the listed ETag the
//! bytes are served without a GET; otherwise the object is fetched and the
//! entry repopulated. Steady state (nothing changed) makes zero GETs, and
//! there is no staleness window.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;

use crate::error::{RecordsError, storage};

/// Bounded number of record objects held in memory.
const CAPACITY: usize = 5000;

struct CachedObject {
    etag: String,
    body: Arc<[u8]>,
}

/// LRU cache of record object bodies, revalidated on S3 ETag.
pub struct RecordCache {
    inner: Mutex<LruCache<String, CachedObject>>,
}

impl Default for RecordCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LruCache::new(NonZeroUsize::new(CAPACITY).expect("nonzero"))),
        }
    }

    /// Return the object's bytes, serving from cache when the cached ETag
    /// matches `etag` (and `etag` is non-empty), otherwise GETting the object
    /// and repopulating the cache.
    ///
    /// The `std::sync::Mutex` guard is never held across an `.await`: the lock
    /// is taken for the map lookup, released before the GET, then re-taken for
    /// the insert. This keeps the method safe under `buffered` concurrency.
    pub async fn get_or_fetch(
        &self,
        s3: &aws_sdk_s3::Client,
        bucket: &str,
        key: &str,
        etag: &str,
    ) -> Result<Arc<[u8]>, RecordsError> {
        // Hit path: matching, non-empty ETag serves cached bytes.
        if !etag.is_empty() {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| RecordsError::CacheUnavailable)?;
            if let Some(entry) = guard.get(key)
                && entry.etag == etag
            {
                return Ok(entry.body.clone());
            }
        }

        // Miss: fetch and repopulate. Guard is dropped before this await.
        let output = claria_storage::objects::get_object(s3, bucket, key)
            .await
            .map_err(|source| storage("reading a record object", source))?;
        let body: Arc<[u8]> = Arc::from(output.body);

        if let Some(fetched_etag) = output.etag {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| RecordsError::CacheUnavailable)?;
            guard.put(
                key.to_string(),
                CachedObject {
                    etag: fetched_etag,
                    body: body.clone(),
                },
            );
        }

        Ok(body)
    }
}
