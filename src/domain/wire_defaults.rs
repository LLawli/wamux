//! Proto3 scalar fields have no presence: an empty string/bytes IS the wire
//! encoding of "not set". waproto fields are proto2 `optional`, so we must map
//! those defaults to `None` and never relay `Some("")` onto the WhatsApp wire.

pub(crate) fn nonempty_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn nonempty_bytes(value: &[u8]) -> Option<Vec<u8>> {
    (!value.is_empty()).then(|| value.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonempty_string_maps_proto3_default_to_none() {
        assert_eq!(nonempty_string(""), None);
        assert_eq!(nonempty_string("x"), Some("x".to_string()));
    }

    #[test]
    fn nonempty_bytes_maps_proto3_default_to_none() {
        assert_eq!(nonempty_bytes(&[]), None);
        assert_eq!(nonempty_bytes(&[7u8]), Some(vec![7u8]));
    }
}
