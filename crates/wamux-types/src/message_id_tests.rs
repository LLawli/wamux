//! `MessageId` (#114): opaque, never empty.

use crate::{MessageId, WamuxError};

#[test]
fn a_message_id_keeps_its_text() {
    let id = MessageId::new("3EB0A1FC2C3F32F320E56A").unwrap();
    assert_eq!(id.as_str(), "3EB0A1FC2C3F32F320E56A");
    assert_eq!(id.to_string(), "3EB0A1FC2C3F32F320E56A");
}

#[test]
fn an_empty_message_id_is_invalid_argument() {
    match MessageId::new("") {
        Err(WamuxError::InvalidArgument(message)) => assert_eq!(message, "empty message id"),
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}
