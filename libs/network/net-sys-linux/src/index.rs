//! Interface name to kernel index resolution.

use std::io;

/// Resolves an interface name to its kernel index.
///
/// # Errors
///
/// Returns [`io::ErrorKind::NotFound`] when the link does not exist and propagates any other operating system error.
pub fn resolve(name: &netmodel::Name) -> io::Result<u32> {
    let text = name.as_str();
    let mut buffer = [0_u8; 16];
    for (slot, byte) in buffer.iter_mut().zip(text.bytes()) {
        *slot = byte;
    }
    // SAFETY: the buffer is 16 bytes, the name is at most 15 bytes, so the
    // copied name is NUL terminated and the pointer is valid for the call.
    let index = unsafe { libc::if_nametoindex(buffer.as_ptr().cast()) };
    if index == 0 {
        return Err(missing(name, io::Error::last_os_error()));
    }

    Ok(index)
}

fn missing(name: &netmodel::Name, error: io::Error) -> io::Error {
    match error.raw_os_error() {
        Some(libc::ENODEV | libc::ENXIO) => io::Error::new(
            io::ErrorKind::NotFound,
            format!("link {name} does not exist"),
        ),
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_finds_loopback() {
        // ARRANGE
        let name = netmodel::Name::new("lo").expect("loopback name");

        // ACT
        let result = resolve(&name);

        // ASSERT
        assert!(result.expect("loopback resolves") > 0);
    }

    #[test]
    fn resolve_rejects_missing_link() {
        // ARRANGE
        let name = netmodel::Name::new("nomissing0").expect("valid name");

        // ACT
        let result = resolve(&name);

        // ASSERT
        assert!(matches!(
            result,
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
    }
}
