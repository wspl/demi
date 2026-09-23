//! Interfaces, addresses and routes through rtnetlink (`managed-hosts.md` §
//! Networking). Each [`session`] opens a netlink socket in the calling
//! thread's network namespace and closes it when its work ends.

use std::{collections::HashMap, future::Future, io, net::Ipv4Addr, os::fd::RawFd};

use futures_util::TryStreamExt;
use ipnet::Ipv4Net;
use rtnetlink::{
    Handle, LinkUnspec, LinkVeth, RouteMessageBuilder,
    packet_route::{
        link::LinkAttribute,
        route::{RouteAddress, RouteAttribute, RouteHeader},
    },
};

#[derive(Debug, thiserror::Error)]
pub enum NetlinkError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Netlink(#[from] rtnetlink::Error),
    #[error("the netlink connection closed")]
    Closed,
}

impl NetlinkError {
    /// Whether the kernel answered that no such device exists.
    fn is_no_device(&self) -> bool {
        match self {
            Self::Netlink(rtnetlink::Error::NetlinkError(message)) => {
                io::Error::from(message.clone()).raw_os_error() == Some(rustix::io::Errno::NODEV.raw_os_error())
            }
            _ => false,
        }
    }
}

/// Runs `work` with a netlink handle; the connection runs beside it and ends
/// with it.
pub async fn session<T, F: Future<Output = Result<T, NetlinkError>>>(
    work: impl FnOnce(Handle) -> F,
) -> Result<T, NetlinkError> {
    let (connection, handle, _unsolicited) = rtnetlink::new_connection()?;
    tokio::select! {
        result = work(handle) => result,
        () = connection => Err(NetlinkError::Closed),
    }
}

/// The IPv4 routes of the main table that name a destination, with the name
/// of their output interface.
pub async fn main_routes(handle: &Handle) -> Result<Vec<(Ipv4Net, Option<String>)>, NetlinkError> {
    let names = link_names(handle).await?;
    let mut routes = Vec::new();
    let mut dump = handle.route().get(RouteMessageBuilder::<Ipv4Addr>::new().build()).execute();
    while let Some(route) = dump.try_next().await? {
        let mut table = u32::from(route.header.table);
        let mut destination = None;
        let mut interface = None;
        for attribute in &route.attributes {
            match attribute {
                RouteAttribute::Table(id) => table = *id,
                RouteAttribute::Destination(RouteAddress::Inet(address)) => destination = Some(*address),
                RouteAttribute::Oif(index) => interface = names.get(index).cloned(),
                _ => {}
            }
        }
        if table != u32::from(RouteHeader::RT_TABLE_MAIN) {
            continue;
        }
        let Some(destination) = destination else {
            continue;
        };
        let network = Ipv4Net::new(destination, route.header.destination_prefix_length)
            .map_err(|error| io::Error::other(error.to_string()))?;
        routes.push((network, interface));
    }
    Ok(routes)
}

async fn link_names(handle: &Handle) -> Result<HashMap<u32, String>, NetlinkError> {
    let mut names = HashMap::new();
    let mut dump = handle.link().get().execute();
    while let Some(link) = dump.try_next().await? {
        for attribute in link.attributes {
            if let LinkAttribute::IfName(name) = attribute {
                names.insert(link.header.index, name);
            }
        }
    }
    Ok(names)
}

/// The index of the interface `name`, `None` when there is none.
pub async fn index(handle: &Handle, name: &str) -> Result<Option<u32>, NetlinkError> {
    let found = handle.link().get().match_name(name.to_owned()).execute().try_next().await;
    match found {
        Ok(link) => Ok(link.map(|link| link.header.index)),
        Err(error) => {
            let error = NetlinkError::from(error);
            if error.is_no_device() {
                return Ok(None);
            }
            Err(error)
        }
    }
}

async fn existing(handle: &Handle, name: &str) -> Result<u32, NetlinkError> {
    index(handle, name).await?.ok_or_else(|| {
        NetlinkError::Io(io::Error::new(io::ErrorKind::NotFound, format!("no interface {name}")))
    })
}

/// Creates the veth pair `host`, `peer`.
pub async fn add_veth(handle: &Handle, host: &str, peer: &str) -> Result<(), NetlinkError> {
    handle.link().add(LinkVeth::new(host, peer).build()).execute().await?;
    Ok(())
}

/// Moves the interface `name` into the network namespace `namespace` refers to.
pub async fn move_to_namespace(handle: &Handle, name: &str, namespace: RawFd) -> Result<(), NetlinkError> {
    let index = existing(handle, name).await?;
    let message = LinkUnspec::new_with_index(index).setns_by_fd(namespace).build();
    handle.link().set(message).execute().await?;
    Ok(())
}

/// Gives the interface `name` the address `address/length` and brings it up.
pub async fn address_and_up(handle: &Handle, name: &str, address: Ipv4Addr, length: u8) -> Result<(), NetlinkError> {
    let index = existing(handle, name).await?;
    handle.address().add(index, address.into(), length).execute().await?;
    up(handle, name).await
}

/// Brings the interface `name` up.
pub async fn up(handle: &Handle, name: &str) -> Result<(), NetlinkError> {
    let index = existing(handle, name).await?;
    handle.link().set(LinkUnspec::new_with_index(index).up().build()).execute().await?;
    Ok(())
}

/// Routes everything through `gateway`.
pub async fn default_route(handle: &Handle, gateway: Ipv4Addr) -> Result<(), NetlinkError> {
    let route = RouteMessageBuilder::<Ipv4Addr>::new().gateway(gateway).build();
    handle.route().add(route).execute().await?;
    Ok(())
}

/// Deletes the interface `name` if it exists; deleting one end of a veth
/// pair removes both.
pub async fn delete(handle: &Handle, name: &str) -> Result<(), NetlinkError> {
    let Some(index) = index(handle, name).await? else {
        return Ok(());
    };
    match handle.link().del(index).execute().await {
        Ok(()) => Ok(()),
        Err(error) => {
            let error = NetlinkError::from(error);
            if error.is_no_device() {
                return Ok(());
            }
            Err(error)
        }
    }
}
