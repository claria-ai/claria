use tantivy::schema::{self, Schema, FAST, INDEXED, STORED, STRING, TEXT};

/// Field names used in the Tantivy index.
pub mod field {
    pub const ID: &str = "id";
    pub const DOC_TYPE: &str = "doc_type";
    pub const ANONYMIZED: &str = "anonymized";
    pub const TITLE: &str = "title";
    pub const BODY: &str = "body";
    pub const S3_KEY: &str = "s3_key";
    pub const CREATED_AT: &str = "created_at";
    pub const UPDATED_AT: &str = "updated_at";
    pub const STATUS: &str = "status";
    pub const MODEL_ID: &str = "model_id";
    pub const TOKEN_COUNT_INPUT: &str = "token_count_input";
    pub const TOKEN_COUNT_OUTPUT: &str = "token_count_output";
    pub const COST_USD: &str = "cost_usd";
    pub const TEMPLATE_ID: &str = "template_id";
    pub const TRANSACTION_ID: &str = "transaction_id";
}

/// Document types stored in the Tantivy index.
pub mod doc_type {
    pub const ASSESSMENT: &str = "assessment";
    pub const SNIPPET: &str = "snippet";
    pub const GOAL: &str = "goal";
    pub const TEMPLATE: &str = "template";
    pub const REPORT: &str = "report";
    pub const TRANSACTION: &str = "transaction";
}

/// Build the Tantivy schema used by the Claria index.
pub fn build_schema() -> Schema {
    let mut builder = Schema::builder();

    // Identifiers — stored and indexed as exact strings
    builder.add_text_field(field::ID, STRING | STORED);
    builder.add_text_field(field::DOC_TYPE, STRING | STORED);

    // Boolean stored as text ("true"/"false") for filtering
    builder.add_text_field(field::ANONYMIZED, STRING | STORED);

    // Full-text searchable fields
    builder.add_text_field(field::TITLE, TEXT | STORED);
    builder.add_text_field(field::BODY, TEXT);

    // Stored-only metadata
    builder.add_text_field(field::S3_KEY, STORED);

    // Timestamps as i64 (Unix seconds) — indexed for range queries, fast for sorting
    builder.add_i64_field(field::CREATED_AT, INDEXED | STORED | FAST);
    builder.add_i64_field(field::UPDATED_AT, INDEXED | STORED | FAST);

    // Filterable string fields
    builder.add_text_field(field::STATUS, STRING | STORED);
    builder.add_text_field(field::MODEL_ID, STRING | STORED);

    // Token counts — stored only
    builder.add_u64_field(field::TOKEN_COUNT_INPUT, STORED);
    builder.add_u64_field(field::TOKEN_COUNT_OUTPUT, STORED);

    // Cost — stored as f64
    builder.add_f64_field(field::COST_USD, STORED);

    // Foreign keys — filterable
    builder.add_text_field(field::TEMPLATE_ID, STRING | STORED);
    builder.add_text_field(field::TRANSACTION_ID, STRING | STORED);

    builder.build()
}

/// Resolve a field by name from the schema, returning the Tantivy `Field` handle.
///
/// # Panics
///
/// Panics if the field name does not exist in the schema. This is only called
/// with compile-time field name constants, so a panic indicates a schema
/// definition bug.
pub fn get_field(schema: &Schema, name: &str) -> schema::Field {
    schema
        .get_field(name)
        .unwrap_or_else(|_| panic!("field '{name}' not found in schema"))
}
