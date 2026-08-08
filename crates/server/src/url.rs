//! Percent-encoding, for the two places the server builds URLs containing text
//! it did not choose: hashtag paths and the `q` carried through search
//! pagination.
//!
//! Hand-rolled rather than pulled in as a dependency. The whole rule is "encode
//! everything outside RFC 3986's unreserved set", which is correct for a path
//! segment and for a query value alike — the difference between the two is only
//! in what you are *allowed* to leave alone, and encoding more than necessary is
//! never wrong.

/// Percent-encodes one path segment or query-string value.
pub fn encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());

    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreserved_characters_pass_through_and_everything_else_does_not() {
        assert_eq!(encode_component("web-dev_2.0~x"), "web-dev_2.0~x");
        assert_eq!(encode_component("a b"), "a%20b");
        // The ones that would otherwise end the value or start a new parameter.
        assert_eq!(encode_component("a&b=c#d"), "a%26b%3Dc%23d");
        // Multi-byte input encodes per byte, not per character.
        assert_eq!(encode_component("é"), "%C3%A9");
    }
}
