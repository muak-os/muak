//! Link lifecycle event construction.

#[cfg(test)]
mod tests {
    use core::num::NonZeroI32;

    use crate::error::LinkError;
    use crate::{LinkEvent, Mac, Name};

    #[test]
    fn added_carries_name_index_and_mac() {
        // ARRANGE
        let name = Name::new("eth0").unwrap();
        let mac = Mac::new([1, 2, 3, 4, 5, 6]);

        // ACT
        let event = LinkEvent::Added {
            name: name.clone(),
            index: 7,
            mac,
        };

        // ASSERT
        assert_eq!(
            event,
            LinkEvent::Added {
                name,
                index: 7,
                mac,
            }
        );
    }

    #[test]
    fn error_carries_typed_failure() {
        // ARRANGE
        let failure = LinkError::Errno(NonZeroI32::new(105_i32).expect("non-zero"));

        // ACT
        let event = LinkEvent::Error(failure);

        // ASSERT
        assert_eq!(event, LinkEvent::Error(failure));
    }

    #[test]
    fn failure_messages_match_the_monitor_contract() {
        // ARRANGE
        let overrun = LinkError::Overrun;
        let errno = LinkError::Errno(NonZeroI32::new(105_i32).expect("non-zero"));
        let closed = LinkError::Closed;

        // ACT
        let overrun_message = overrun.to_string();
        let errno_message = errno.to_string();
        let closed_message = closed.to_string();

        // ASSERT
        assert_eq!(overrun_message, "event buffer overran, events may be lost");
        assert_eq!(errno_message, "error code 105");
        assert_eq!(closed_message, "event stream closed");
    }
}
