//! claria-search
//!
//! Tantivy index lifecycle: download from S3, query, mutate, flush back with ETag locking.

pub mod error;
pub mod flush;
pub mod index;
pub mod mutate;
pub mod query;
