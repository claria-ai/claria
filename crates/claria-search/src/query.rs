use tantivy::collector::TopDocs;
use tantivy::query::{QueryParser, TermQuery};
use tantivy::schema::{IndexRecordOption, Value};
use tantivy::{Index, Term};

use claria_core::schema::{field, get_field};

use crate::error::SearchError;

/// A retrieved document from the index.
pub struct SearchResult {
    pub id: String,
    pub doc_type: String,
    pub title: String,
    pub s3_key: String,
    pub score: f32,
}

/// Full-text search across title and body fields.
pub fn search(
    index: &Index,
    query_text: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();

    let title_field = get_field(&schema, field::TITLE);
    let body_field = get_field(&schema, field::BODY);

    let query_parser = QueryParser::for_index(index, vec![title_field, body_field]);
    let query = query_parser
        .parse_query(query_text)
        .map_err(|e| SearchError::QueryParse(e.to_string()))?;

    let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

    let id_field = get_field(&schema, field::ID);
    let doc_type_field = get_field(&schema, field::DOC_TYPE);
    let s3_key_field = get_field(&schema, field::S3_KEY);

    let mut results = Vec::new();
    for (score, doc_address) in top_docs {
        let doc = searcher.doc::<tantivy::TantivyDocument>(doc_address)?;

        let id = doc
            .get_first(id_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let doc_type = doc
            .get_first(doc_type_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let title = doc
            .get_first(title_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let s3_key = doc
            .get_first(s3_key_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        results.push(SearchResult {
            id,
            doc_type,
            title,
            s3_key,
            score,
        });
    }

    Ok(results)
}

/// Find all documents of a given type.
pub fn find_by_type(
    index: &Index,
    doc_type: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();

    let doc_type_field = get_field(&schema, field::DOC_TYPE);
    let query = TermQuery::new(
        Term::from_field_text(doc_type_field, doc_type),
        IndexRecordOption::Basic,
    );

    let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

    let id_field = get_field(&schema, field::ID);
    let title_field = get_field(&schema, field::TITLE);
    let s3_key_field = get_field(&schema, field::S3_KEY);

    let mut results = Vec::new();
    for (score, doc_address) in top_docs {
        let doc = searcher.doc::<tantivy::TantivyDocument>(doc_address)?;

        let id = doc
            .get_first(id_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let title = doc
            .get_first(title_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let s3_key = doc
            .get_first(s3_key_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        results.push(SearchResult {
            id,
            doc_type: doc_type.to_string(),
            title,
            s3_key,
            score,
        });
    }

    Ok(results)
}

/// Find a single document by ID.
pub fn find_by_id(index: &Index, id: &str) -> Result<Option<tantivy::TantivyDocument>, SearchError> {
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();

    let id_field = get_field(&schema, field::ID);
    let query = TermQuery::new(
        Term::from_field_text(id_field, id),
        IndexRecordOption::Basic,
    );

    let top_docs = searcher.search(&query, &TopDocs::with_limit(1))?;

    if let Some((_score, doc_address)) = top_docs.first() {
        let doc = searcher.doc::<tantivy::TantivyDocument>(*doc_address)?;
        Ok(Some(doc))
    } else {
        Ok(None)
    }
}
