use anyhow::{Context, Result};
use futures::stream::TryStreamExt;
use netlink_packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use rtnetlink::Handle;
use rtnetlink::LinkUnspec;

pub async fn find_link_by_name(handle: &Handle, name: &str) -> Result<LinkMessage> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    links
        .try_next()
        .await
        .context("failed to query link")?
        .ok_or_else(|| anyhow::anyhow!("link '{}' not found", name))
}

pub async fn get_link_index(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_link_by_name(handle, name).await?;
    Ok(link.header.index)
}

pub async fn link_exists(handle: &Handle, name: &str) -> Result<bool> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    match links.try_next().await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(_) => Ok(false),
    }
}

pub async fn bring_link_up(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await
        .context("failed to bring link up")
}

pub async fn bring_link_down(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).down().build())
        .execute()
        .await
        .context("failed to bring link down")
}

pub async fn ensure_link_up(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_link_by_name(handle, name).await?;
    let index = link.header.index;

    if !link.header.flags.contains(LinkFlags::Up) {
        bring_link_up(handle, index).await?;
    }

    Ok(index)
}

pub fn extract_mac_from_link(link: &LinkMessage) -> Option<[u8; 6]> {
    for attr in &link.attributes {
        if let LinkAttribute::Address(addr) = attr
            && addr.len() == 6
        {
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&addr[..6]);
            return Some(mac);
        }
    }
    None
}

pub async fn set_link_master(handle: &Handle, slave_index: u32, master_index: u32) -> Result<()> {
    handle
        .link()
        .set(
            LinkUnspec::new_with_index(slave_index)
                .controller(master_index)
                .build(),
        )
        .execute()
        .await
        .context("failed to set link master")
}

pub async fn delete_link(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .del(index)
        .execute()
        .await
        .context("failed to delete link")
}

pub async fn unset_link_master(handle: &Handle, slave_index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(slave_index).nocontroller().build())
        .execute()
        .await
        .context("failed to unset link master")
}

pub fn is_link_flag_up(link: &LinkMessage) -> bool {
    // Check both IFF_UP flag and carrier state
    // For enslaved interfaces, carrier reflects actual physical link state
    let has_up_flag = link.header.flags.contains(LinkFlags::Up);
    
    // Check carrier attribute (1 = carrier present, 0 = no carrier)
    let has_carrier = link.attributes.iter().any(|attr| {
        matches!(attr, LinkAttribute::Carrier(1))
    });
    
    // Link is considered up if it has both UP flag and carrier
    // OR if it has UP flag and no Carrier attribute (legacy behavior)
    has_up_flag && (has_carrier || !link.attributes.iter().any(|attr| matches!(attr, LinkAttribute::Carrier(_))))
}

pub fn has_master(link: &LinkMessage) -> bool {
    link.attributes.iter().any(|attr| matches!(attr, LinkAttribute::Controller(_)))
}

pub fn extract_name_from_link(link: &LinkMessage) -> Option<String> {
    for attr in &link.attributes {
        if let LinkAttribute::IfName(name) = attr {
            return Some(name.clone());
        }
    }
    None
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_link(flags: LinkFlags, attrs: Vec<LinkAttribute>) -> LinkMessage {
        let mut msg = LinkMessage::default();
        msg.header.flags = flags;
        msg.header.index = 1;
        msg.attributes = attrs;
        msg
    }

    // ========================================================================
    // is_link_flag_up() Tests
    // ========================================================================

    #[test]
    fn test_link_up_with_carrier() {
        let link = create_test_link(
            LinkFlags::Up | LinkFlags::Running,
            vec![LinkAttribute::Carrier(1)],
        );
        assert!(is_link_flag_up(&link), "Link with UP flag and carrier=1 should be up");
    }

    #[test]
    fn test_link_up_without_carrier() {
        let link = create_test_link(
            LinkFlags::Up | LinkFlags::Running,
            vec![LinkAttribute::Carrier(0)],
        );
        assert!(!is_link_flag_up(&link), "Link with UP flag but carrier=0 should be down");
    }

    #[test]
    fn test_link_down_with_carrier() {
        let link = create_test_link(
            LinkFlags::empty(),
            vec![LinkAttribute::Carrier(1)],
        );
        assert!(!is_link_flag_up(&link), "Link without UP flag should be down even with carrier");
    }

    #[test]
    fn test_link_up_no_carrier_attribute() {
        // Legacy behavior: if no Carrier attribute, trust UP flag
        let link = create_test_link(
            LinkFlags::Up | LinkFlags::Running,
            vec![LinkAttribute::IfName("eth0".to_string())],
        );
        assert!(is_link_flag_up(&link), "Link with UP flag and no carrier attr should be up");
    }

    #[test]
    fn test_link_down_no_carrier_attribute() {
        let link = create_test_link(
            LinkFlags::empty(),
            vec![LinkAttribute::IfName("eth0".to_string())],
        );
        assert!(!is_link_flag_up(&link), "Link without UP flag should be down");
    }

    // ========================================================================
    // has_master() Tests
    // ========================================================================

    #[test]
    fn test_has_master_with_controller() {
        let link = create_test_link(
            LinkFlags::Up,
            vec![LinkAttribute::Controller(5)], // master index 5
        );
        assert!(has_master(&link), "Link with Controller attribute should have master");
    }

    #[test]
    fn test_has_master_without_controller() {
        let link = create_test_link(
            LinkFlags::Up,
            vec![
                LinkAttribute::IfName("eth0".to_string()),
                LinkAttribute::Carrier(1),
            ],
        );
        assert!(!has_master(&link), "Link without Controller attribute should not have master");
    }

    #[test]
    fn test_has_master_empty_attributes() {
        let link = create_test_link(LinkFlags::Up, vec![]);
        assert!(!has_master(&link), "Link with empty attributes should not have master");
    }

    // ========================================================================
    // extract_mac_from_link() Tests
    // ========================================================================

    #[test]
    fn test_extract_mac_valid() {
        let mac_bytes = vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let link = create_test_link(
            LinkFlags::Up,
            vec![LinkAttribute::Address(mac_bytes)],
        );
        let mac = extract_mac_from_link(&link);
        assert_eq!(mac, Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]));
    }

    #[test]
    fn test_extract_mac_wrong_length() {
        // 4-byte address (not a MAC)
        let link = create_test_link(
            LinkFlags::Up,
            vec![LinkAttribute::Address(vec![1, 2, 3, 4])],
        );
        assert_eq!(extract_mac_from_link(&link), None);
    }

    #[test]
    fn test_extract_mac_no_address() {
        let link = create_test_link(
            LinkFlags::Up,
            vec![LinkAttribute::IfName("eth0".to_string())],
        );
        assert_eq!(extract_mac_from_link(&link), None);
    }

    // ========================================================================
    // extract_name_from_link() Tests
    // ========================================================================

    #[test]
    fn test_extract_name_valid() {
        let link = create_test_link(
            LinkFlags::Up,
            vec![LinkAttribute::IfName("eth0".to_string())],
        );
        assert_eq!(extract_name_from_link(&link), Some("eth0".to_string()));
    }

    #[test]
    fn test_extract_name_missing() {
        let link = create_test_link(
            LinkFlags::Up,
            vec![LinkAttribute::Carrier(1)],
        );
        assert_eq!(extract_name_from_link(&link), None);
    }

    #[test]
    fn test_extract_name_multiple_attributes() {
        let link = create_test_link(
            LinkFlags::Up,
            vec![
                LinkAttribute::Carrier(1),
                LinkAttribute::IfName("enp0s3".to_string()),
                LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6]),
            ],
        );
        assert_eq!(extract_name_from_link(&link), Some("enp0s3".to_string()));
    }
}
