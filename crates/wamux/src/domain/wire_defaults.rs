//! Proto3 scalar fields have no presence: an empty string/bytes IS the wire
//! encoding of "not set". waproto fields are proto2 `optional`, so we must map
//! those defaults to `None` and never relay `Some("")` onto the WhatsApp wire.

pub(crate) fn nonempty_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn nonempty_bytes(value: &[u8]) -> Option<Vec<u8>> {
    (!value.is_empty()).then(|| value.to_vec())
}

pub(crate) fn nonzero_u32(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}

pub(crate) fn nonzero_i32(value: i32) -> Option<i32> {
    (value != 0).then_some(value)
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

    #[test]
    fn nonzero_scalars_map_proto3_default_to_none() {
        assert_eq!(nonzero_u32(0), None);
        assert_eq!(nonzero_u32(90), Some(90));
        assert_eq!(nonzero_i32(0), None);
        assert_eq!(nonzero_i32(1), Some(1));
    }
}
