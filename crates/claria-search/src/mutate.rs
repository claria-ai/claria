use tantivy::{Index, IndexWriter, Term};

use claria_core::schema::{field, get_field};

use crate::error::SearchError;

/// Insert a new document into the index.
///
/// The caller provides field values as a `TantivyDocument`. The `id` field
/// must be set — it is used for deduplication.
pub fn insert_document(
    writer: &IndexWriter,
    doc: tantivy::TantivyDocument,
) -> Result<(), SearchError> {
    writer.add_document(doc)?;
    Ok(())
}

/// Delete a document by ID, then insert the replacement.
/// This is the standard "update" pattern in Tantivy.
pub fn update_document(
    index: &Index,
    writer: &IndexWriter,
    id: &str,
    doc: tantivy::TantivyDocument,
) -> Result<(), SearchError> {
    let schema = index.schema();
    let id_field = get_field(&schema, field::ID);
    let term = Term::from_field_text(id_field, id);

    writer.delete_term(term);
    writer.add_document(doc)?;
    Ok(())
}

/// Delete a document by ID.
pub fn delete_document(index: &Index, writer: &IndexWriter, id: &str) -> Result<(), SearchError> {
    let schema = index.schema();
    let id_field = get_field(&schema, field::ID);
    let term = Term::from_field_text(id_field, id);

    writer.delete_term(term);
    Ok(())
}

/// Commit all pending changes to the index.
pub fn commit(writer: &mut IndexWriter) -> Result<(), SearchError> {
    writer.commit()?;
    Ok(())
}
