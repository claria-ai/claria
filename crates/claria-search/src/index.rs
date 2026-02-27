use std::path::{Path, PathBuf};

use aws_sdk_s3::Client;
use tantivy::Index;
use tracing::info;

use claria_core::s3_keys;
use claria_core::schema::build_schema;
use claria_storage::objects;

use crate::error::SearchError;

/// A loaded Tantivy index with its S3 ETag for optimistic locking.
pub struct LoadedIndex {
    pub index: Index,
    pub index_dir: PathBuf,
    pub etag: String,
}

/// Download the Tantivy index from S3 and open it.
///
/// The index is stored as `_index/tantivy.tar.zst` in the bucket.
/// It is downloaded, decompressed, and extracted to `dest_dir`.
pub async fn download_index(
    client: &Client,
    bucket: &str,
    dest_dir: &Path,
) -> Result<LoadedIndex, SearchError> {
    info!("downloading Tantivy index from s3://{}/{}", bucket, s3_keys::INDEX);

    let output = objects::get_object(client, bucket, s3_keys::INDEX)
        .await
        .map_err(|e| match e {
            claria_storage::error::StorageError::NotFound { .. } => SearchError::IndexNotFound,
            other => SearchError::Storage(other),
        })?;

    let etag = output.etag.unwrap_or_default();

    // Decompress zstd
    let decoder = zstd::Decoder::new(output.body.as_slice())?;

    // Extract tar archive
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest_dir)?;

    info!("index extracted to {:?}, etag={}", dest_dir, etag);

    let index = Index::open_in_dir(dest_dir)
        .map_err(|e| SearchError::IndexCorrupted(e.to_string()))?;

    Ok(LoadedIndex {
        index,
        index_dir: dest_dir.to_path_buf(),
        etag,
    })
}

/// Create a new empty Tantivy index in the given directory.
pub fn create_empty_index(dest_dir: &Path) -> Result<Index, SearchError> {
    let schema = build_schema();
    let index = Index::create_in_dir(dest_dir, schema)?;
    Ok(index)
}
