//! Helpers for building AWS-style XML response bodies.

/// Wrap a body in an XML declaration.
pub fn xml_doc(body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{body}")
}

/// Simple XML element: `<Tag>value</Tag>`.
pub fn el(tag: &str, value: &str) -> String {
    format!("<{tag}>{value}</{tag}>")
}

/// Wrap children in a parent tag with an xmlns attribute.
pub fn wrap_ns(tag: &str, xmlns: &str, children: &str) -> String {
    format!("<{tag} xmlns=\"{xmlns}\">{children}</{tag}>")
}

/// Wrap children in a parent tag.
pub fn wrap(tag: &str, children: &str) -> String {
    format!("<{tag}>{children}</{tag}>")
}

/// Build a REST-XML (S3) error response body, where `Error` is the root.
pub fn error_xml(code: &str, message: &str) -> String {
    xml_doc(&wrap(
        "Error",
        &format!("{}{}", el("Code", code), el("Message", message)),
    ))
}

/// Build a query-protocol (IAM, STS) error response body.
///
/// These services nest `Error` inside an `ErrorResponse` envelope. The SDK's
/// generated parser only reads `Code` from a child element named `Error`, so a
/// bare `Error` root deserializes to an unhandled error with no code at all —
/// which is how a `LimitExceeded` reaches the caller as "service error".
pub fn query_error_xml(code: &str, message: &str) -> String {
    xml_doc(&wrap(
        "ErrorResponse",
        &wrap(
            "Error",
            &format!(
                "{}{}{}",
                el("Type", "Sender"),
                el("Code", code),
                el("Message", message)
            ),
        ),
    ))
}
