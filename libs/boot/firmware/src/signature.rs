//! EFI signature list queries.

/// X.509 certificate signature type GUID used by EFI signature lists.
const EFI_CERT_X509_GUID: uefi::Guid = uefi::guid!("a5c059a1-94e4-4aa7-87b5-ab155c2bf072");

/// Size of an EFI signature list header in bytes.
const SIGNATURE_LIST_HEADER_SIZE: usize = 28;

/// Size of an EFI signature data header in bytes.
const SIGNATURE_DATA_HEADER_SIZE: usize = 16;

/// Returns whether `data` contains an X.509 signature list with `certificate`.
#[must_use = "callers must branch on whether the key is enrolled"]
pub fn contains_x509(data: &[u8], certificate: &[u8]) -> bool {
    if certificate.is_empty() {
        return false;
    }

    let x509 = EFI_CERT_X509_GUID.to_bytes();
    let mut offset = 0_usize;

    while let Some(header_end) = offset.checked_add(SIGNATURE_LIST_HEADER_SIZE) {
        if header_end > data.len() {
            break;
        }

        let Some(list_size) =
            read_u32(data, offset.saturating_add(16)).and_then(|size| usize::try_from(size).ok())
        else {
            return false;
        };
        let Some(end) = offset.checked_add(list_size) else {
            return false;
        };
        if list_size < SIGNATURE_LIST_HEADER_SIZE || end > data.len() {
            return false;
        }

        let Some(list) = data.get(offset..end) else {
            return false;
        };
        if list.get(..x509.len()) == Some(x509.as_slice()) && x509_list_contains(list, certificate)
        {
            return true;
        }

        offset = end;
    }

    false
}

fn x509_list_contains(list: &[u8], certificate: &[u8]) -> bool {
    let Some(signature_size) = read_u32(list, 24)
        .and_then(|size| usize::try_from(size).ok())
        .filter(|&size| size > 0)
    else {
        return false;
    };
    let Some(header_size) = read_u32(list, 20).and_then(|size| usize::try_from(size).ok()) else {
        return false;
    };
    let Some(certificate_size) = signature_size.checked_sub(SIGNATURE_DATA_HEADER_SIZE) else {
        return false;
    };
    let Some(mut offset) = SIGNATURE_LIST_HEADER_SIZE.checked_add(header_size) else {
        return false;
    };

    while let Some(end) = offset.checked_add(signature_size) {
        if end > list.len() {
            break;
        }

        let Some(certificate_start) = offset.checked_add(SIGNATURE_DATA_HEADER_SIZE) else {
            return false;
        };
        let Some(certificate_end) = certificate_start.checked_add(certificate_size) else {
            return false;
        };
        if list.get(certificate_start..certificate_end) == Some(certificate) {
            return true;
        }

        offset = end;
    }

    false
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes = data.get(offset..end)?;

    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x509_signature_list(certificate: &[u8]) -> Vec<u8> {
        let signature_size = SIGNATURE_DATA_HEADER_SIZE
            .checked_add(certificate.len())
            .expect("signature size");
        let list_size = SIGNATURE_LIST_HEADER_SIZE
            .checked_add(signature_size)
            .expect("list size");
        let mut list = Vec::with_capacity(list_size);
        list.extend_from_slice(&EFI_CERT_X509_GUID.to_bytes());
        list.extend_from_slice(&u32::try_from(list_size).expect("list size").to_le_bytes());
        list.extend_from_slice(&0_u32.to_le_bytes());
        list.extend_from_slice(
            &u32::try_from(signature_size)
                .expect("signature size")
                .to_le_bytes(),
        );
        list.extend_from_slice(&[0_u8; 16]);
        list.extend_from_slice(certificate);

        list
    }

    #[test]
    fn finds_x509_certificate_in_signature_list() {
        // ARRANGE
        let certificate = b"our-x509-certificate";
        let data = x509_signature_list(certificate);

        // ACT
        let found = contains_x509(&data, certificate);
        let other = contains_x509(&data, b"foreign-x509-certificate");
        let empty = contains_x509(&data, b"");

        // ASSERT
        assert!(found, "the enrolled certificate must be found");
        assert!(!other, "a different certificate must not match");
        assert!(!empty, "an empty certificate must not match");
    }

    #[test]
    fn ignores_non_x509_and_malformed_lists() {
        // ARRANGE
        let mut non_x509 = x509_signature_list(b"certificate");
        non_x509
            .get_mut(..16)
            .expect("signature type bytes")
            .copy_from_slice(&[0_u8; 16]);
        let malformed = [0_u8; SIGNATURE_LIST_HEADER_SIZE];

        // ACT
        let skipped = contains_x509(&non_x509, b"certificate");
        let invalid = contains_x509(&malformed, b"certificate");

        // ASSERT
        assert!(!skipped, "non-X509 lists must not count as certificates");
        assert!(!invalid, "an invalid list size must not match");
    }
}
