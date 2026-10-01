//! Network slots (`setup.md` § Configuration): each running sandbox takes the
//! lowest free /30 of the pool, which names its interfaces and namespace.

use std::{cell::RefCell, collections::BTreeSet, net::Ipv4Addr, rc::Rc};

use ipnet::Ipv4Net;

/// One sandbox's network: the /30 at `index` in the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub index: u16,
    /// The host's address, the sandbox's gateway.
    pub gateway: Ipv4Addr,
    /// The sandbox's address.
    pub address: Ipv4Addr,
}

impl Slot {
    /// The slot at `index` of `subnet`, which the configuration made large
    /// enough for every index below its slot count.
    pub fn at(subnet: Ipv4Net, index: u16) -> Self {
        let first = u32::from(subnet.network()) + u32::from(index) * 4;
        Self {
            index,
            gateway: Ipv4Addr::from(first + 1),
            address: Ipv4Addr::from(first + 2),
        }
    }

    /// The host end of the sandbox's veth pair, such as `demih3`.
    pub fn host_interface(&self) -> String {
        format!("demih{}", self.index)
    }

    /// The sandbox's end of the veth pair, such as `demip3`.
    pub fn peer_interface(&self) -> String {
        format!("demip{}", self.index)
    }

    /// The sandbox's network namespace, such as `demi-3`.
    pub fn namespace(&self) -> String {
        format!("demi-{}", self.index)
    }
}

/// The slots of the configured pool.
pub struct SlotPool {
    subnet: Ipv4Net,
    count: u16,
    taken: Rc<RefCell<BTreeSet<u16>>>,
}

/// Every slot is in use.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("All Cloud network slots are in use")]
pub struct Exhausted;

/// A slot a running sandbox holds; dropping it frees the slot.
#[derive(Debug)]
pub struct SlotLease {
    slot: Slot,
    taken: Rc<RefCell<BTreeSet<u16>>>,
}

impl SlotPool {
    pub fn new(subnet: Ipv4Net, count: u16) -> Self {
        Self {
            subnet,
            count,
            taken: Rc::default(),
        }
    }

    /// The slot at `index`, taken or not, as a record names it.
    pub fn slot(&self, index: u16) -> Slot {
        Slot::at(self.subnet, index)
    }

    /// Whether `index` lies in the configured pool.
    pub fn contains(&self, index: u16) -> bool {
        index < self.count
    }

    /// Takes the lowest free slot.
    pub fn take(&self) -> Result<SlotLease, Exhausted> {
        let mut taken = self.taken.borrow_mut();
        let index = (0..self.count)
            .find(|index| !taken.contains(index))
            .ok_or(Exhausted)?;
        taken.insert(index);
        Ok(SlotLease {
            slot: self.slot(index),
            taken: self.taken.clone(),
        })
    }
}

impl SlotLease {
    pub fn slot(&self) -> &Slot {
        &self.slot
    }
}

impl Drop for SlotLease {
    fn drop(&mut self) {
        self.taken.borrow_mut().remove(&self.slot.index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_have_disjoint_networks_and_are_reused_after_release() {
        let pool = SlotPool::new("172.30.0.0/16".parse().unwrap(), 3);
        let a = pool.take().expect("slot 0");
        let b = pool.take().expect("slot 1");
        assert_eq!(
            (a.slot().index, a.slot().gateway, a.slot().address),
            (0, Ipv4Addr::new(172, 30, 0, 1), Ipv4Addr::new(172, 30, 0, 2))
        );
        assert_eq!(
            (b.slot().index, b.slot().gateway, b.slot().address),
            (1, Ipv4Addr::new(172, 30, 0, 5), Ipv4Addr::new(172, 30, 0, 6))
        );
        let far = pool.slot(64);
        assert_eq!((far.gateway, far.address), (Ipv4Addr::new(172, 30, 1, 1), Ipv4Addr::new(172, 30, 1, 2)));
        assert_eq!(
            (far.host_interface(), far.peer_interface(), far.namespace()),
            ("demih64".into(), "demip64".into(), "demi-64".into())
        );
        let c = pool.take().expect("slot 2");
        assert_eq!(pool.take().expect_err("exhausted").to_string(), "All Cloud network slots are in use");
        let second = b.slot().clone();
        drop(b);
        let reused = pool.take().expect("slot 1 again");
        assert_eq!(*reused.slot(), second);
        drop((a, c, reused));
        assert!(pool.contains(2) && !pool.contains(3));
    }
}
