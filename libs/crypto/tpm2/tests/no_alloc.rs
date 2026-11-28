#![cfg(feature = "linux")]

//! Proof that the TPM2 streaming paths never allocate.

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tpm2::blob::Sealed;
    use tpm2::error::{Result, Tpm2Error};
    use tpm2::operations;
    use tpm2::transport::Transport;

    const RESPONSE_TAG: u16 = 0x8001;
    const RESPONSE_HEADER_SIZE: usize = 10;
    const MAX_RESPONSE: usize = 64;
    const NONCE_SIZE: usize = 32;
    const OUTPUT_SIZE: usize = 64;
    const PCR: [u8; 32] = [0xAA; 32];
    const DATA: [u8; 1] = [9];
    const PUBLIC: [u8; 2] = [3, 4];
    const PRIVATE: [u8; 2] = [1, 2];

    const HANDLE_EXISTS_FOUND_BODY: [u8; 13] = [0, 0, 0, 0, 1, 0, 0, 0, 1, 0x81, 0, 0, 1];
    const CREATE_BODY: [u8; 12] = [0, 0, 0, 8, 0, 2, 1, 2, 0, 2, 3, 4];
    const LOAD_BODY: [u8; 4] = [0x80, 0, 0, 2];
    const SESSION_BODY: [u8; 4] = [0x03, 0, 0, 0];
    const UNSEAL_BODY: [u8; 7] = [0, 0, 0, 3, 0, 1, 0xAA];
    const EMPTY_BODY: [u8; 0] = [];

    static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

    #[global_allocator]
    static GLOBAL: Counting = Counting;

    struct Counting;

    // SAFETY: every method forwards to the system allocator with the same layout it received, and the counter update is an atomic that cannot allocate or unwind.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: the layout is passed through unchanged to the system allocator, which honours the same contract.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: the layout is passed through unchanged to the system allocator, which honours the same contract.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: the pointer, layout and new size are passed through unchanged to the system allocator, which honours the same contract.
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the pointer and layout came from this allocator, which forwards them unchanged to the system allocator.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    struct NoAllocTransport<'a> {
        responses: &'a [&'a [u8]],
        index: usize,
    }

    impl<'a> NoAllocTransport<'a> {
        const fn new(responses: &'a [&'a [u8]]) -> Self {
            Self {
                responses,
                index: 0,
            }
        }
    }

    impl Transport for NoAllocTransport<'_> {
        fn transact(&mut self, _request: &[u8]) -> Result<&[u8]> {
            let response =
                self.responses
                    .get(self.index)
                    .copied()
                    .ok_or(Tpm2Error::ResponseTooShort {
                        expected: RESPONSE_HEADER_SIZE,
                        actual: 0,
                    })?;
            self.index = self.index.saturating_add(1);

            Ok(response)
        }
    }

    fn fill_response<'a>(buffer: &'a mut [u8; MAX_RESPONSE], body: &[u8]) -> &'a [u8] {
        let size = RESPONSE_HEADER_SIZE
            .checked_add(body.len())
            .expect("response size should fit usize");
        let (tag_slot, rest) = buffer.split_at_mut(2);
        let (size_slot, rest) = rest.split_at_mut(4);
        let (rc_slot, rest) = rest.split_at_mut(4);
        tag_slot.copy_from_slice(&RESPONSE_TAG.to_be_bytes());
        size_slot.copy_from_slice(&u32::try_from(size).ok().unwrap_or(0).to_be_bytes());
        rc_slot.copy_from_slice(&0_u32.to_be_bytes());
        let body_slot = rest
            .get_mut(..body.len())
            .expect("body should fit the buffer");
        body_slot.copy_from_slice(body);

        buffer.get(..size).expect("response range should exist")
    }

    fn delta(before: usize) -> usize {
        ALLOCATIONS.load(Ordering::SeqCst).saturating_sub(before)
    }

    #[test]
    fn allocation_free_paths_do_not_allocate() {
        // ARRANGE
        let mut handle_exists_buffer = [0_u8; MAX_RESPONSE];
        let handle_exists = fill_response(&mut handle_exists_buffer, &HANDLE_EXISTS_FOUND_BODY);
        let mut create_buffer = [0_u8; MAX_RESPONSE];
        let create = fill_response(&mut create_buffer, &CREATE_BODY);
        let mut load_buffer = [0_u8; MAX_RESPONSE];
        let load = fill_response(&mut load_buffer, &LOAD_BODY);
        let mut session_buffer = [0_u8; MAX_RESPONSE];
        let session = fill_response(&mut session_buffer, &SESSION_BODY);
        let mut empty_buffer = [0_u8; MAX_RESPONSE];
        let empty = fill_response(&mut empty_buffer, &EMPTY_BODY);
        let mut unseal_buffer = [0_u8; MAX_RESPONSE];
        let unseal = fill_response(&mut unseal_buffer, &UNSEAL_BODY);

        let seal_responses: [&[u8]; 2] = [handle_exists, create];
        let unseal_responses: [&[u8]; 7] =
            [handle_exists, load, session, empty, unseal, empty, empty];
        let blob = Sealed::try_new(&PUBLIC, &PRIVATE).expect("small sealed blob should be valid");
        let mut wire = [0_u8; Sealed::MAX_SERIALIZED_SIZE];
        let mut out = [0_u8; OUTPUT_SIZE];
        let nonce = [0x55_u8; NONCE_SIZE];
        let device_path = Path::new("/dev/tpmrm0");

        {
            let mut transport = NoAllocTransport::new(&seal_responses);
            let warmup = operations::seal(&mut transport, &DATA, &PCR);
            assert!(warmup.is_ok(), "seal warmup should succeed");
        }
        {
            let mut transport = NoAllocTransport::new(&unseal_responses);
            let warmup = operations::unseal(&mut transport, &blob, &nonce, &mut out);
            assert!(warmup.is_ok(), "unseal warmup should succeed");
        }
        let serialized = blob
            .serialize_into(&mut wire)
            .expect("sealed blob should serialize");
        let warmup = Sealed::deserialize(serialized);
        assert!(warmup.is_ok(), "deserialize warmup should succeed");
        let _warmup_available = tpm2::linux::is_available(Some(device_path));

        let control_before = ALLOCATIONS.load(Ordering::SeqCst);
        drop(Vec::<u8>::with_capacity(1));
        let control_allocations = delta(control_before);
        assert!(
            control_allocations >= 1,
            "counting allocator must observe allocations, saw {control_allocations}"
        );

        // ACT
        let before = ALLOCATIONS.load(Ordering::SeqCst);
        let seal_result = {
            let mut transport = NoAllocTransport::new(&seal_responses);
            operations::seal(&mut transport, &DATA, &PCR)
        };
        let seal_allocations = delta(before);

        let before = ALLOCATIONS.load(Ordering::SeqCst);
        let unseal_result = {
            let mut transport = NoAllocTransport::new(&unseal_responses);
            operations::unseal(&mut transport, &blob, &nonce, &mut out)
        };
        let unseal_allocations = delta(before);

        let before = ALLOCATIONS.load(Ordering::SeqCst);
        let deserialize_result = Sealed::deserialize(serialized);
        let deserialize_allocations = delta(before);

        let before = ALLOCATIONS.load(Ordering::SeqCst);
        let serialize_result = blob.serialize_into(&mut wire);
        let serialize_allocations = delta(before);

        let before = ALLOCATIONS.load(Ordering::SeqCst);
        let available = tpm2::linux::is_available(Some(device_path));
        let availability_allocations = delta(before);

        // ASSERT
        assert!(seal_result.is_ok(), "seal should succeed");
        assert_eq!(
            seal_allocations, 0,
            "operations::seal made {seal_allocations} allocations"
        );
        assert!(unseal_result.is_ok(), "unseal should succeed");
        assert_eq!(
            unseal_allocations, 0,
            "operations::unseal made {unseal_allocations} allocations"
        );
        assert!(deserialize_result.is_ok(), "deserialize should succeed");
        assert_eq!(
            deserialize_allocations, 0,
            "Sealed::deserialize made {deserialize_allocations} allocations"
        );
        assert!(serialize_result.is_ok(), "serialize should succeed");
        assert_eq!(
            serialize_allocations, 0,
            "Sealed::serialize_into made {serialize_allocations} allocations"
        );
        assert_eq!(
            availability_allocations, 0,
            "linux::is_available made {availability_allocations} allocations"
        );
        assert_eq!(
            available,
            std::fs::metadata(device_path).is_ok(),
            "is_available should match the device path"
        );
    }
}
