//! claria-records
//!
//! Client records stored in S3: client CRUD and name history, the record
//! inventory (the S3 walk behind the sidecar-visibility rules in
//! `claria_core::s3_keys::visible_record_files`), record-text fetching through
//! an ETag-revalidated cache, and the retryable, compensating delete/restore
//! lifecycle for a client's data.
//!
//! `claria-report-authoring` deliberately does not depend on this crate (the
//! lifecycle here depends on it, and the edge must not be a cycle); it applies
//! the same visibility rules through the pure helper in `claria-core`.

pub mod cache;
pub mod clients;
pub mod error;
pub mod inventory;
pub mod lifecycle;

pub use cache::RecordCache;
pub use claria_storage::objects::S3_FETCH_CONCURRENCY;
pub use clients::{
    ClientNameHistoryEntry, ClientNameUpdate, ClientRecordDetails, ClientSummary,
    get_client_record_details, list_client_summaries, update_client_name, validate_client_name,
};
pub use error::RecordsError;
pub use inventory::{
    RecordInventoryEntry, fetch_record_text, fetch_record_texts, record_inventory,
    search_record_contents,
};
pub use lifecycle::{ClientDeletionOutcome, ClientRestoreOutcome, delete_client, restore_client};
