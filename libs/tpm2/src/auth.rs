//! TPM2 authorization helpers.

use crate::buffer::CommandBuffer;
use crate::handles::SessionHandle;

const TPM2_RS_PW: u32 = 0x4000_0009;
const EMPTY_NONCE_SIZE: u16 = 0;
const EMPTY_HMAC_SIZE: u16 = 0;
const PASSWORD_SESSION_ATTRIBUTES: u8 = 1;
const POLICY_SESSION_ATTRIBUTES: u8 = 0;
const SINGLE_AUTH_AREA_SIZE: u32 = 9;

pub(crate) enum AuthArea {
    Password,
    Policy(SessionHandle),
}

impl AuthArea {
    pub(crate) fn encode_sized(&self, command: &mut CommandBuffer) {
        command.write_u32(SINGLE_AUTH_AREA_SIZE);
        match *self {
            Self::Password => {
                command.write_u32(TPM2_RS_PW);
                command.write_u16(EMPTY_NONCE_SIZE);
                command.write_u8(PASSWORD_SESSION_ATTRIBUTES);
                command.write_u16(EMPTY_HMAC_SIZE);
            }
            Self::Policy(session) => {
                command.write_handle(session);
                command.write_u16(EMPTY_NONCE_SIZE);
                command.write_u8(POLICY_SESSION_ATTRIBUTES);
                command.write_u16(EMPTY_HMAC_SIZE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_auth_area_matches_expected_wire_format() {
        // ARRANGE
        let mut command = CommandBuffer::new(0, 0);

        // ACT
        AuthArea::Password.encode_sized(&mut command);
        let finalized = command.finalize();

        // ASSERT
        assert!(finalized.is_ok(), "password auth area should encode");
        let finalized = finalized.ok().unwrap_or_default();
        assert_eq!(
            finalized.get(10..23),
            Some(&[0, 0, 0, 9, 0x40, 0, 0, 9, 0, 0, 1, 0, 0][..]),
            "password auth should match wire format"
        );
    }

    #[test]
    fn policy_auth_area_matches_expected_wire_format() {
        // ARRANGE
        let mut command = CommandBuffer::new(0, 0);

        // ACT
        AuthArea::Policy(SessionHandle::from(0x0300_0000)).encode_sized(&mut command);
        let finalized = command.finalize();

        // ASSERT
        assert!(finalized.is_ok(), "policy auth area should encode");
        let finalized = finalized.ok().unwrap_or_default();
        assert_eq!(
            finalized.get(10..23),
            Some(&[0, 0, 0, 9, 0x03, 0, 0, 0, 0, 0, 0, 0, 0][..]),
            "policy auth should match wire format"
        );
    }
}
