use ring::digest;

pub fn generate_mac_address(vm_id: &str) -> [u8; 6] {
    let result = digest::digest(&digest::SHA256, vm_id.as_bytes());

    let mut mac = [0u8; 6];
    mac.copy_from_slice(&result.as_ref()[0..6]);

    // Set the locally administered bit and clear the multicast bit
    // Bit 1 = locally administered, Bit 0 = unicast/multicast
    mac[0] = (mac[0] & 0xfe) | 0x02;

    mac
}

pub fn format_mac_address(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}
