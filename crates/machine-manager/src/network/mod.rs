//! Sandbox networking (`managed-hosts.md` § Networking): a network namespace
//! and veth pair per running sandbox, configured through netlink, and one
//! firewall table for all of them.

#[cfg(target_os = "linux")]
mod netlink;
#[cfg(target_os = "linux")]
mod netns;
pub mod ruleset;
pub mod slots;

#[cfg(target_os = "linux")]
pub use linux::{CloudNetwork, NetworkError};

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        io,
        net::Ipv4Addr,
        os::fd::AsRawFd,
        path::PathBuf,
    };

    use ipnet::Ipv4Net;

    use super::{
        netlink::{self, NetlinkError},
        netns,
        ruleset::{self, Policy},
        slots::Slot,
    };
    use crate::{
        blocking,
        linux::thread_ns::{self, Namespace},
    };

    /// The prefix of every host interface of a Cloud slot.
    const HOST_INTERFACES: &str = "demih";

    /// The kernel's IPv6 settings, absent when it runs without IPv6, such as
    /// when booted with `ipv6.disable=1`.
    const IPV6_SETTINGS: &str = "/proc/sys/net/ipv6";

    #[derive(Debug, thiserror::Error)]
    pub enum NetworkError {
        #[error(transparent)]
        Io(#[from] io::Error),
        #[error(transparent)]
        Netlink(#[from] NetlinkError),
        #[error("Cannot apply the Cloud firewall: {0}")]
        Firewall(String),
        #[error("DEMI_MANAGED_SUBNET overlaps host route {0}")]
        Overlap(Ipv4Net),
    }

    /// Applies `ruleset` as one `nft` transaction; a refusal carries what
    /// `nft` printed.
    fn apply(
        nft: &std::path::Path,
        ruleset: &nftables::schema::Nftables<'_>,
    ) -> Result<(), NetworkError> {
        use nftables::helper::{DEFAULT_ARGS, NftablesError, apply_ruleset_with_args};
        apply_ruleset_with_args(ruleset, Some(nft), DEFAULT_ARGS).map_err(|error| match error {
            NftablesError::NftFailed { stderr, .. } => {
                NetworkError::Firewall(stderr.trim().to_owned())
            }
            other => NetworkError::Firewall(other.to_string()),
        })
    }

    /// The host side of Cloud networking.
    #[derive(Debug, Clone)]
    pub struct CloudNetwork {
        pool: Ipv4Net,
        dns: Vec<Ipv4Addr>,
        nft: PathBuf,
    }

    impl CloudNetwork {
        pub fn new(pool: Ipv4Net, dns: Vec<Ipv4Addr>, nft: PathBuf) -> Self {
            Self { pool, dns, nft }
        }

        /// Checks that the pool overlaps no host route, enables forwarding,
        /// and installs the firewall table.
        pub async fn prepare(&self) -> Result<(), NetworkError> {
            self.check().await?;
            let table = ruleset::table(&Policy {
                pool: self.pool,
                dns: &self.dns,
            });
            let nft = self.nft.clone();
            blocking::run(move |_| -> Result<(), NetworkError> {
                fs_err::write("/proc/sys/net/ipv4/ip_forward", "1")?;
                apply(&nft, &table)
            })
            .await
        }

        /// Checks that the pool overlaps no host route, changing nothing.
        pub async fn check(&self) -> Result<(), NetworkError> {
            let routes =
                netlink::session(|handle| async move { netlink::main_routes(&handle).await })
                    .await?;
            for (network, interface) in routes {
                let default = network.prefix_len() == 0;
                let ours = interface.is_some_and(|name| name.starts_with(HOST_INTERFACES));
                if default || ours {
                    continue;
                }
                if self.pool.contains(&network.network()) || network.contains(&self.pool.network())
                {
                    return Err(NetworkError::Overlap(network));
                }
            }
            Ok(())
        }

        /// Creates `slot`'s namespace and veth pair, gives both ends their
        /// addresses with IPv6 off, routes the sandbox through the host end,
        /// and admits the slot's pair to the firewall. A kernel without IPv6
        /// has none to turn off on either end.
        pub async fn attach(&self, slot: &Slot) -> Result<(), NetworkError> {
            let namespace = slot.namespace();
            let host = slot.host_interface();
            let peer = slot.peer_interface();
            netns::create(&namespace).await?;
            let file = netns::path(&namespace);
            let opened = blocking::run({
                let file = file.clone();
                move |_| fs_err::File::open(file)
            })
            .await?;
            let gateway = slot.gateway;
            netlink::session(|handle| {
                let (host, peer) = (host.clone(), peer.clone());
                async move {
                    netlink::add_veth(&handle, &host, &peer).await?;
                    netlink::move_to_namespace(&handle, &peer, opened.as_raw_fd()).await?;
                    netlink::address_and_up(&handle, &host, gateway, 30).await
                }
            })
            .await?;
            let ipv6 = blocking::run({
                let host = host.clone();
                move |_| -> io::Result<bool> {
                    if !std::path::Path::new(IPV6_SETTINGS).try_exists()? {
                        return Ok(false);
                    }
                    fs_err::write(format!("{IPV6_SETTINGS}/conf/{host}/disable_ipv6"), "1")?;
                    Ok(true)
                }
            })
            .await?;
            let address = slot.address;
            thread_ns::run(Namespace::Network(file), move |_| {
                if ipv6 {
                    fs_err::write(format!("{IPV6_SETTINGS}/conf/all/disable_ipv6"), "1")?;
                }
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .build()?;
                runtime
                    .block_on(netlink::session(|handle| async move {
                        netlink::up(&handle, "lo").await?;
                        netlink::address_and_up(&handle, &peer, address, 30).await?;
                        netlink::default_route(&handle, gateway).await
                    }))
                    .map_err(io::Error::other)
            })
            .await?;
            let element = ruleset::add_slot(slot);
            let nft = self.nft.clone();
            blocking::run(move |_| apply(&nft, &element)).await
        }

        /// Removes `slot`'s firewall pair, veth pair and namespace, each only
        /// if it exists.
        pub async fn detach(&self, slot: &Slot) -> Result<(), NetworkError> {
            let removal = ruleset::remove_slot(slot);
            let nft = self.nft.clone();
            blocking::run(move |_| apply(&nft, &removal)).await?;
            let host = slot.host_interface();
            netlink::session(|handle| async move { netlink::delete(&handle, &host).await }).await?;
            let namespace = slot.namespace();
            blocking::run(move |off| netns::remove(off, &namespace)).await?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use std::process::Command;

        use super::*;
        use crate::{linux::testing::isolate, network::slots::SlotPool};

        fn output(program: &str, args: &[&str]) -> String {
            let output = Command::new(program).args(args).output().expect(program);
            assert!(
                output.status.success(),
                "{program} {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        }

        fn read(path: &str) -> String {
            std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}"))
        }

        #[tokio::test]
        #[ignore = "needs root: run the Linux suite with --ignored as root"]
        async fn a_slot_attaches_and_detaches_under_the_installed_policy() {
            // Before anything starts a thread: they inherit these namespaces.
            isolate();
            let nft = which::which("nft").unwrap();
            let network = CloudNetwork::new(
                "172.30.0.0/16".parse().unwrap(),
                vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)],
                nft,
            );
            network.prepare().await.unwrap();
            assert_eq!(read("/proc/sys/net/ipv4/ip_forward").trim(), "1");
            let listing = output("nft", &["list", "table", "inet", "demi_cloud"]);
            let expected = include_str!("../../tests/fixtures/nft-ruleset.txt");
            assert_eq!(listing, expected, "the installed table:\n{listing}");
            // Installing it again replaces it.
            network.prepare().await.unwrap();
            assert_eq!(
                output("nft", &["list", "table", "inet", "demi_cloud"]),
                expected
            );

            let slot = SlotPool::new("172.30.0.0/16".parse().unwrap(), 8).slot(3);
            network.attach(&slot).await.unwrap();
            let set = output("nft", &["list", "set", "inet", "demi_cloud", "slots"]);
            assert!(set.contains("\"demih3\" . 172.30.0.14"), "{set}");
            assert!(output("ip", &["-o", "addr", "show", "demih3"]).contains("172.30.0.13/30"));
            let inside = output("ip", &["-n", "demi-3", "-o", "addr", "show", "demip3"]);
            assert!(inside.contains("172.30.0.14/30"), "{inside}");
            let routes = output("ip", &["-n", "demi-3", "route"]);
            assert!(routes.contains("default via 172.30.0.13"), "{routes}");
            // IPv6 is off on both ends, unless the kernel runs without it.
            if std::path::Path::new(IPV6_SETTINGS).exists() {
                assert_eq!(
                    read("/proc/sys/net/ipv6/conf/demih3/disable_ipv6").trim(),
                    "1"
                );
                let disabled = output(
                    "ip",
                    &[
                        "netns",
                        "exec",
                        "demi-3",
                        "cat",
                        "/proc/sys/net/ipv6/conf/all/disable_ipv6",
                    ],
                );
                assert_eq!(disabled.trim(), "1");
            }

            network.detach(&slot).await.unwrap();
            let set = output("nft", &["list", "set", "inet", "demi_cloud", "slots"]);
            assert!(!set.contains("demih3"), "{set}");
            assert!(!std::path::Path::new("/sys/class/net/demih3").exists());
            assert!(!std::path::Path::new("/run/netns/demi-3").exists());
            // Detaching what is gone is fine, with or without the table.
            network.detach(&slot).await.unwrap();
            output("nft", &["delete", "table", "inet", "demi_cloud"]);
            network.detach(&slot).await.unwrap();
        }

        #[tokio::test]
        #[ignore = "needs root: run the Linux suite with --ignored as root"]
        async fn a_pool_that_overlaps_a_host_route_is_refused() {
            isolate();
            output("ip", &["link", "set", "lo", "up"]);
            // A veth pair, which the manager needs anyway: a kernel may lack
            // the dummy driver.
            output(
                "ip",
                &[
                    "link", "add", "probe0", "type", "veth", "peer", "name", "probe1",
                ],
            );
            output("ip", &["addr", "add", "172.30.5.1/24", "dev", "probe0"]);
            output("ip", &["link", "set", "probe0", "up"]);
            let nft = which::which("nft").unwrap();
            let overlapping = CloudNetwork::new(
                "172.30.0.0/16".parse().unwrap(),
                vec![Ipv4Addr::new(1, 1, 1, 1)],
                nft,
            );
            let error = overlapping.prepare().await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "DEMI_MANAGED_SUBNET overlaps host route 172.30.5.0/24"
            );
        }
    }
}
