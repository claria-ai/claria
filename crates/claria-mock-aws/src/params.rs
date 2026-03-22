/// Extract a value from a `key=value&key2=value2` string (form-encoded or query string).
pub fn extract(params: &str, key: &str) -> Option<String> {
    for pair in params.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return Some(
                percent_encoding::percent_decode_str(v)
                    .decode_utf8_lossy()
                    .to_string(),
            );
        }
    }
    None
}
