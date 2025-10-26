use crate::log;
use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

pub struct IpAllocator {
    pool_start: u32,
    pool_end: u32,
    allocated: Arc<Mutex<HashSet<u32>>>,
}

impl IpAllocator {
    pub fn new(start: Ipv4Addr, end: Ipv4Addr) -> Self {
        let pool_start = u32::from_be_bytes(start.octets());
        let pool_end = u32::from_be_bytes(end.octets());

        Self {
            pool_start,
            pool_end,
            allocated: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn allocate(&self) -> Result<Ipv4Addr, Box<dyn std::error::Error>> {
        let mut allocated = self.allocated.lock().unwrap();

        for ip_u32 in self.pool_start..=self.pool_end {
            if !allocated.contains(&ip_u32) {
                allocated.insert(ip_u32);
                let ip = Ipv4Addr::from(ip_u32.to_be_bytes());
                log!("network", "Allocated IP: {}", ip);
                return Ok(ip);
            }
        }

        Err("No available IP addresses in pool".into())
    }

    pub fn release(&self, ip: Ipv4Addr) -> Result<(), Box<dyn std::error::Error>> {
        let ip_u32 = u32::from_be_bytes(ip.octets());
        let mut allocated = self.allocated.lock().unwrap();

        if allocated.remove(&ip_u32) {
            log!("network", "Released IP: {}", ip);
            Ok(())
        } else {
            Err(format!("IP {} was not allocated", ip).into())
        }
    }

    pub fn is_allocated(&self, ip: Ipv4Addr) -> bool {
        let ip_u32 = u32::from_be_bytes(ip.octets());
        let allocated = self.allocated.lock().unwrap();
        allocated.contains(&ip_u32)
    }

    pub fn allocated_count(&self) -> usize {
        let allocated = self.allocated.lock().unwrap();
        allocated.len()
    }

    pub fn pool_size(&self) -> usize {
        (self.pool_end - self.pool_start + 1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_release() {
        let allocator =
            IpAllocator::new(Ipv4Addr::new(10, 42, 0, 10), Ipv4Addr::new(10, 42, 0, 20));

        let ip1 = allocator.allocate().unwrap();
        assert_eq!(ip1, Ipv4Addr::new(10, 42, 0, 10));

        let ip2 = allocator.allocate().unwrap();
        assert_eq!(ip2, Ipv4Addr::new(10, 42, 0, 11));

        allocator.release(ip1).unwrap();

        let ip3 = allocator.allocate().unwrap();
        assert_eq!(ip3, Ipv4Addr::new(10, 42, 0, 10));
    }
}
