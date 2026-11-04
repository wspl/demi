//! Sandbox networking: a dedicated network namespace and veth pair per running
//! sandbox, configured through netlink, from which `runsc` builds its isolated
//! stack, and one firewall table for all of them (`ruleset`). There are no
//! guest taps and no per-user host port publications.
//!
//! The policy:
//!
//! - It allows the Claude Code machine public destinations, replies to
//!   established connections and the configured resolver on TCP and UDP
//!   port 53.
//! - It allows a Cloud what the network rules allow, which the backend names
//!   each time it connects and after each change
//!   (`crates/machine-manager-protocol/src/network.rs`): public destinations,
//!   private networks and addresses by the firewall, domains by the
//!   resolver a Cloud's DNS is redirected to (`resolver`). The manager keeps
//!   the rules in memory only, so it starts with the internet alone until
//!   the backend connects.
//! - It rejects every other destination, the host's own services included,
//!   on forwarded traffic and on traffic addressed to the host. No rule opens
//!   loopback, link-local, metadata, multicast, the host or another Cloud.
//! - The backend is not among the allowed: a Cloud reaches it through the
//!   runner socket, never the network.
//!
//! No other infrastructure transport is an alternate route for conversation
//! operations: the runner's connection carries files, commands and browser
//! streams through normal Host access.
//!
//! Further rules:
//!
//! - IPv6 is off, on the host end of each veth pair and in the sandbox's
//!   namespace, so that it cannot bypass the IPv4 policy. A host whose kernel
//!   runs without IPv6 has none to turn off, and a boot there skips that step.
//! - The resolver is an address reachable from the sandbox, never a container
//!   engine's loopback resolver. Filtering is by destination address, so DNS
//!   rebinding does not grant private-network access. An exception for a
//!   private resolver is an exact endpoint, not a whole subnet, and an address
//!   change needs revalidation and a rule update.
//! - Another service's firewall is not rewritten. Docker, for example, can
//!   leave a `FORWARD` policy of `DROP`, which an accepting rule in this table
//!   does not override, so an operator configures that service's integration
//!   rules for the Cloud interfaces and verifies public egress and private
//!   refusal through a real sandbox.

#[cfg(target_os = "linux")]
mod netlink;
#[cfg(target_os = "linux")]
mod netns;
#[cfg(target_os = "linux")]
mod resolver;
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
        rc::Rc,
    };

    use demi_machine_manager_protocol::{NetworkRules, SandboxMachine};
    use ipnet::Ipv4Net;

    use super::{
        netlink::{self, NetlinkError},
        netns,
        resolver::Resolver,
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
    pub(super) fn apply(
        nft: &std::path::Path,
        ruleset: &nftables::schema::Nftables<'_>,
    ) -> Result<(), NetworkError> {
        run_nft(nft, ruleset, &[])
    }

    /// Has `nft` read `ruleset` and check it against the kernel, applying
    /// nothing, so a kernel that lacks what the table uses refuses it before
    /// the manager starts.
    fn check_ruleset(
        nft: &std::path::Path,
        ruleset: &nftables::schema::Nftables<'_>,
    ) -> Result<(), NetworkError> {
        run_nft(nft, ruleset, &["-c"])
    }

    /// The oldest nftables that reads the table's JSON: 0.9.0 refuses its
    /// concatenated set type, and says nothing of why.
    const NFT_OLDEST: [u32; 3] = [0, 9, 3];

    /// Refuses an `nft` older than [`NFT_OLDEST`], by what `nft --version`
    /// prints, `nftables v0.9.3 (Topsy)`. A version it does not print so
    /// leaves the judgment to [`check_ruleset`].
    fn check_version(nft: &std::path::Path) -> Result<(), NetworkError> {
        let output = std::process::Command::new(nft)
            .arg("--version")
            .output()
            .map_err(|error| NetworkError::Firewall(format!("running {}: {error}", nft.display())))?;
        let printed = String::from_utf8_lossy(&output.stdout);
        let Some(version) = printed.split_whitespace().find_map(|word| word.strip_prefix('v')) else {
            return Ok(());
        };
        let Ok(parts) = version.split('.').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>() else {
            return Ok(());
        };
        if parts.as_slice() < NFT_OLDEST.as_slice() {
            return Err(NetworkError::Firewall(format!(
                "nftables {version} cannot read the firewall's JSON; 0.9.3 or later reads it"
            )));
        }
        Ok(())
    }

    /// Runs `nft -j -f -` with `args` first on `ruleset`. Not through the
    /// `nftables` crate's helper, which leaves nft's standard error to the
    /// manager's, so a refusal would say nothing of why.
    fn run_nft(
        nft: &std::path::Path,
        ruleset: &nftables::schema::Nftables<'_>,
        args: &[&str],
    ) -> Result<(), NetworkError> {
        use std::io::Write as _;
        let payload = serde_json::to_vec(ruleset).expect("a ruleset serializes");
        let mut child = std::process::Command::new(nft)
            .args(args)
            .args(["-j", "-f", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| NetworkError::Firewall(format!("running {}: {error}", nft.display())))?;
        let written = child.stdin.take().expect("stdin is piped").write_all(&payload);
        let output = child
            .wait_with_output()
            .map_err(|error| NetworkError::Firewall(format!("running {}: {error}", nft.display())))?;
        if output.status.success() {
            return Ok(());
        }
        written.map_err(|error| NetworkError::Firewall(format!("writing to {}: {error}", nft.display())))?;
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("; ");
        Err(NetworkError::Firewall(said))
    }

    /// The host side of Cloud networking.
    #[derive(Debug, Clone)]
    pub struct CloudNetwork {
        pool: Ipv4Net,
        dns: Vec<Ipv4Addr>,
        nft: PathBuf,
        resolver: Rc<Resolver>,
    }

    impl CloudNetwork {
        pub fn new(pool: Ipv4Net, dns: Vec<Ipv4Addr>, nft: PathBuf) -> Self {
            let resolver = Resolver::new(dns.clone(), nft.clone());
            Self { pool, dns, nft, resolver }
        }

        /// The resolvers a sandbox's `/etc/resolv.conf` names. The Claude
        /// Code machine asks them; a Cloud's questions to them, as to any
        /// other address, reach its gateway's resolver.
        pub fn dns(&self) -> &[Ipv4Addr] {
            &self.dns
        }

        /// Checks that the pool overlaps no host route, enables forwarding,
        /// and installs the firewall table with what `rules` allows Clouds
        /// and the pairs of `running`, the slots of the sandboxes that keep
        /// running, each running Cloud with its resolver.
        pub async fn prepare(
            &self,
            running: &[(Slot, SandboxMachine)],
            rules: &NetworkRules,
        ) -> Result<(), NetworkError> {
            self.check().await?;
            for (slot, machine) in running {
                if *machine == SandboxMachine::Cloud {
                    self.resolver.listen(slot).await?;
                }
            }
            let _firewall = self.resolver.firewall().await;
            let named = self.resolver.named(rules);
            let table = ruleset::table(
                &Policy {
                    pool: self.pool,
                    dns: &self.dns,
                    rules,
                    named: &named,
                },
                running,
            );
            let nft = self.nft.clone();
            blocking::run(move |_| -> Result<(), NetworkError> {
                fs_err::write("/proc/sys/net/ipv4/ip_forward", "1")?;
                apply(&nft, &table)
            })
            .await?;
            self.resolver.set_rules(rules);
            Ok(())
        }

        /// Mounts the directory of the slots' namespace files, private to
        /// the manager's mount namespace; once, after the saved namespace is
        /// pinned and before the first attach.
        pub async fn mount_namespace_directory(&self) -> Result<(), NetworkError> {
            Ok(blocking::run(netns::mount_directory).await?)
        }

        /// The file that holds `slot`'s network namespace.
        pub fn namespace_file(&self, slot: &Slot) -> PathBuf {
            netns::path(&slot.namespace())
        }

        /// Checks that the pool overlaps no host route and that `nft` takes
        /// the firewall table, changing nothing.
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
            let table = ruleset::table(
                &Policy {
                    pool: self.pool,
                    dns: &self.dns,
                    rules: &NetworkRules::default(),
                    named: &[],
                },
                &[],
            );
            let nft = self.nft.clone();
            blocking::run(move |_| {
                check_version(&nft)?;
                check_ruleset(&nft, &table)
            })
            .await
        }

        /// Creates `slot`'s namespace and veth pair, gives both ends their
        /// addresses with IPv6 off, routes the sandbox through the host end,
        /// and admits the slot's pair to the firewall as a `machine`'s. A
        /// kernel without IPv6 has none to turn off on either end.
        pub async fn attach(&self, slot: &Slot, machine: SandboxMachine) -> Result<(), NetworkError> {
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
            let element = ruleset::add_slot(slot, machine);
            let nft = self.nft.clone();
            blocking::run(move |_| apply(&nft, &element)).await?;
            if machine == SandboxMachine::Cloud {
                self.resolver.listen(slot).await?;
            }
            Ok(())
        }

        /// Removes `slot`'s resolver, firewall pair, veth pair and namespace,
        /// each only if it exists.
        pub async fn detach(&self, slot: &Slot) -> Result<(), NetworkError> {
            self.resolver.stop(slot);
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

}
