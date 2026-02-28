//! S3 key/path conventions.
//!
//! Pure string functions — no AWS SDK dependency. These define the canonical
//! layout of objects in the Claria S3 bucket.

use uuid::Uuid;

pub fn assessment(id: Uuid) -> String {
    format!("assessments/{id}.json")
}

pub fn snippet(id: Uuid) -> String {
    format!("snippets/{id}.json")
}

pub fn goal(id: Uuid) -> String {
    format!("goals/{id}.json")
}

pub fn template(id: Uuid) -> String {
    format!("templates/{id}.tera")
}

pub fn report_answer(id: Uuid) -> String {
    format!("reports/{id}/answer.json")
}

pub fn report_docx(id: Uuid) -> String {
    format!("reports/{id}/report.docx")
}

pub fn report_pdf(id: Uuid) -> String {
    format!("reports/{id}/report.pdf")
}

pub fn report_transaction(id: Uuid) -> String {
    format!("reports/{id}/transaction.json")
}

pub fn client(id: Uuid) -> String {
    format!("clients/{id}.json")
}

pub const CLIENTS_PREFIX: &str = "clients/";

pub fn client_records_prefix(id: Uuid) -> String {
    format!("records/{id}/")
}

pub fn client_record_file(id: Uuid, filename: &str) -> String {
    format!("records/{id}/{filename}")
}

pub const SYSTEM_PROMPT: &str = "system-prompt.md";

pub const INDEX: &str = "_index/tantivy.tar.zst";

pub const PROVISIONER_STATE: &str = "_state/provisioner.json";
