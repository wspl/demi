//! The firewall: one fixed nftables table whose rules are the same for every
//! sandbox.
//!
//! Its only per-sandbox facts are elements of two sets of the pair of a
//! sandbox's host interface and source address: `slots`, every sandbox's, so
//! a packet from a Cloud interface with any other pair is dropped and a
//! workload can neither replace the rules nor send as another sandbox, and
//! `clouds`, the Clouds' alone, which the network rules the master's policy
//! gives (`crates/machine-manager-protocol/src/network.rs`) apply to; the
//! Claude Code machine reaches the internet alone.
//!
//! A Cloud's every DNS question, to whatever address, is redirected to the
//! manager's resolver on its slot's gateway (`super::resolver`), which adds
//! the addresses of an allowed name's answer to `named`, or to `named_ports`
//! with each port its entries name, before the Cloud gets the answer. Their
//! elements expire; an open connection outlives them.
//!
//! A denied destination is rejected, not dropped, with ICMP administratively
//! prohibited, so a connection fails at once with "No route to host" rather
//! than waiting for a timeout that says nothing of why.
//!
//! The table is installed whole whenever the manager reconciles, with the pair
//! of every sandbox that runs and the names' addresses that have not expired,
//! in the same transaction, since a reconcile keeps those sandboxes running.
//! For example, a backend that restarts sends `reconcile` while the Claude Code
//! machine runs a request: without its pair the new table would drop every
//! packet the Cloud sends, and the request would wait on DNS that never
//! answers while the Cloud looks healthy.
//!
//! The table is built as typed data with the `nftables` crate and applied with
//! `nft` in one transaction: the netlink libraries for nftables are thinly
//! maintained, and a text script would be assembled from strings.
//!
//! ```text
//! table inet demi_cloud {
//!   set slots  { type ifname . ipv4_addr; }
//!   set clouds { type ifname . ipv4_addr; }
//!   set named  { type ipv4_addr; flags timeout; }
//!   set named_ports { type ipv4_addr . inet_service; flags timeout; }
//!   chain resolver { nat hook prerouting dstnat; iifname . ip saddr @clouds
//!                    meta l4proto {tcp, udp} th dport 53 redirect to :RESOLVER_PORT }
//!   chain input   { filter hook input -10; iifname "demih*" jump ingress }
//!   chain forward { filter hook forward -10; iifname "demih*" jump ingress;
//!                   oifname "demih*" ct state established,related accept; oifname "demih*" drop }
//!   chain ingress { meta nfproto != ipv4 drop; iifname . ip saddr != @slots drop;
//!                   iifname . ip saddr @clouds meta l4proto {tcp, udp}
//!                       th dport RESOLVER_PORT fib daddr type local accept;
//!                   ip daddr {dns} meta l4proto {tcp, udp} th dport 53 accept;
//!                   fib daddr type local reject; ip daddr {never, pool} reject;
//!                   ct state established,related accept;
//!                   iifname . ip saddr != @clouds ip daddr {denied} reject;
//!                   iifname . ip saddr != @clouds accept;
//!                   <one rule for each group or destination the rules allow>;
//!                   ip daddr @named accept;
//!                   meta l4proto {tcp, udp} ip daddr . th dport @named_ports accept;
//!                   reject }
//!   chain nat     { nat hook postrouting srcnat; ip saddr <pool> oifname != "demih*" masquerade }
//! }
//! ```

use std::{borrow::Cow, collections::{BTreeMap, HashSet}, net::Ipv4Addr};

use demi_machine_manager_protocol::{
    Allowed, NetworkRules, SandboxMachine,
    network::{NEVER_REACHABLE, PRIVATE_NETWORKS},
};
use ipnet::Ipv4Net;
use nftables::{
    batch::Batch,
    expr::{
        CT, Elem, Expression, Fib, FibFlag, FibResult, Meta, MetaKey, NamedExpression, Payload,
        PayloadField, Prefix, SetItem,
    },
    schema::{Chain, Element, NfListObject, Nftables, Rule, Set, SetFlag, SetType, SetTypeValue, Table},
    stmt::{JumpTarget, Match, NAT, Operator, Reject, RejectType, Statement},
    types::{NfChainPolicy, NfChainType, NfFamily, NfHook, RejectCode},
};

use super::slots::Slot;

pub const TABLE: &str = "demi_cloud";
/// Every sandbox's pair.
const SLOTS: &str = "slots";
/// The pair of each sandbox that is a Cloud.
const CLOUDS: &str = "clouds";
/// The addresses of allowed names, on every port.
const NAMED: &str = "named";
/// The addresses of allowed names with the port their entry names.
const NAMED_PORTS: &str = "named_ports";
/// Every Cloud host interface's name starts with this.
const INTERFACES: &str = "demih*";
/// The filter chains run just before the default priority.
const FILTER_PRIORITY: i32 = -10;
/// `srcnat`: the priority of source NAT in the postrouting hook.
const SOURCE_NAT_PRIORITY: i32 = 100;
/// `dstnat`: the priority of destination NAT in the prerouting hook.
const DESTINATION_NAT_PRIORITY: i32 = -100;
/// The port the Clouds' resolver listens on, on each slot's gateway, which a
/// Cloud's questions to port 53 are redirected to. Not 53 itself: a resolver
/// of the host's own, such as dnsmasq listening on every address, may hold
/// it, and the manager could not listen there.
pub const RESOLVER_PORT: u16 = 25_353;

/// Destinations a sandbox may not reach unless an exact rule before this
/// one allows it: private, shared, loopback, link-local (metadata),
/// documentation, multicast and reserved ranges. The benchmarking range,
/// `198.18.0.0/15`, counts as the internet: a proxy in fake-IP mode, such as
/// Clash's, answers every name with an address there and carries the
/// connection to the real site, so a server whose resolver is such a proxy
/// would otherwise reach nothing.
const DENIED: [(Ipv4Addr, u32); 13] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

/// What sandboxes reach: the Claude Code machine the resolvers and public
/// destinations, a Cloud its gateway's resolver and what the rules and the
/// names' addresses allow. The backend is none of them: a Cloud reaches it
/// through its runner socket (`crates/backend-http/src/lib.rs`).
pub struct Policy<'a> {
    pub pool: Ipv4Net,
    pub dns: &'a [Ipv4Addr],
    pub rules: &'a NetworkRules,
    pub named: &'a [Named],
}

/// An address an allowed name's answer gave, on every port or on one, for
/// `timeout` seconds more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named {
    pub address: Ipv4Addr,
    pub port: Option<u16>,
    pub timeout: u32,
}

/// The whole table, replacing any table of that name in one transaction:
/// adding it first makes the deletion succeed whether or not it existed.
/// The new sets hold the pairs of each slot in `running`, whose sandbox keeps
/// running across the replacement.
pub fn table(policy: &Policy<'_>, running: &[(Slot, SandboxMachine)]) -> Nftables<'static> {
    let mut batch = Batch::new();
    batch.add(NfListObject::Table(table_object()));
    batch.delete(NfListObject::Table(table_object()));
    batch.add(NfListObject::Table(table_object()));
    for name in [SLOTS, CLOUDS] {
        batch.add(set(name));
    }
    for (slot, machine) in running {
        for name in sets_of(*machine) {
            batch.add(NfListObject::Element(element(name, slot)));
        }
    }
    batch.add(named_set(NAMED, SetTypeValue::Single(SetType::Ipv4Addr)));
    batch.add(named_set(
        NAMED_PORTS,
        SetTypeValue::Concatenated(Cow::Owned(vec![SetType::Ipv4Addr, SetType::InetService])),
    ));
    for element in named_elements(policy.named) {
        batch.add(NfListObject::Element(element));
    }
    let chains = [
        base_chain(
            "resolver",
            NfChainType::NAT,
            NfHook::Prerouting,
            DESTINATION_NAT_PRIORITY,
        ),
        base_chain("input", NfChainType::Filter, NfHook::Input, FILTER_PRIORITY),
        base_chain(
            "forward",
            NfChainType::Filter,
            NfHook::Forward,
            FILTER_PRIORITY,
        ),
        regular_chain("ingress"),
        base_chain(
            "nat",
            NfChainType::NAT,
            NfHook::Postrouting,
            SOURCE_NAT_PRIORITY,
        ),
    ];
    for chain in chains {
        batch.add(NfListObject::Chain(chain));
    }
    let jump = || {
        Statement::Jump(JumpTarget {
            target: "ingress".into(),
        })
    };
    let from_clouds = || {
        matches(
            Expression::Named(NamedExpression::Concat(vec![meta(MetaKey::Iifname), payload("ip", "saddr")])),
            text(&format!("@{CLOUDS}")),
        )
    };
    let not_from_clouds = || {
        differs(
            Expression::Named(NamedExpression::Concat(vec![meta(MetaKey::Iifname), payload("ip", "saddr")])),
            text(&format!("@{CLOUDS}")),
        )
    };
    let port = |port: u16| {
        [
            matches(meta(MetaKey::L4proto), set_of(vec![text("tcp"), text("udp")])),
            matches(payload("th", "dport"), Expression::Number(u32::from(port))),
        ]
    };
    let local = || {
        matches(
            Expression::Named(NamedExpression::Fib(Fib {
                result: FibResult::Type,
                flags: HashSet::from([FibFlag::Daddr]),
            })),
            text("local"),
        )
    };
    let rules = [
        (
            "resolver",
            [
                vec![from_clouds()],
                port(53).to_vec(),
                vec![Statement::Redirect(Some(NAT {
                    addr: None,
                    family: None,
                    port: Some(Expression::Number(u32::from(RESOLVER_PORT))),
                    flags: None,
                }))],
            ]
            .concat(),
        ),
        (
            "input",
            vec![matches(meta(MetaKey::Iifname), text(INTERFACES)), jump()],
        ),
        (
            "forward",
            vec![matches(meta(MetaKey::Iifname), text(INTERFACES)), jump()],
        ),
        (
            "forward",
            vec![
                matches(meta(MetaKey::Oifname), text(INTERFACES)),
                Statement::Match(Match {
                    left: Expression::Named(NamedExpression::CT(CT {
                        key: "state".into(),
                        family: None,
                        dir: None,
                    })),
                    right: Expression::List(vec![text("established"), text("related")]),
                    op: Operator::IN,
                }),
                Statement::Accept(None),
            ],
        ),
        (
            "forward",
            vec![
                matches(meta(MetaKey::Oifname), text(INTERFACES)),
                Statement::Drop(None),
            ],
        ),
        (
            "ingress",
            vec![
                differs(meta(MetaKey::Nfproto), text("ipv4")),
                Statement::Drop(None),
            ],
        ),
        (
            "ingress",
            vec![
                differs(
                    Expression::Named(NamedExpression::Concat(vec![
                        meta(MetaKey::Iifname),
                        payload("ip", "saddr"),
                    ])),
                    text(&format!("@{SLOTS}")),
                ),
                Statement::Drop(None),
            ],
        ),
        (
            "ingress",
            [vec![from_clouds()], port(RESOLVER_PORT).to_vec(), vec![local(), Statement::Accept(None)]].concat(),
        ),
        (
            "ingress",
            [vec![matches(payload("ip", "daddr"), addresses(policy.dns))], port(53).to_vec(), vec![Statement::Accept(None)]]
                .concat(),
        ),
        ("ingress", vec![local(), reject()]),
        (
            "ingress",
            vec![
                matches(
                    payload("ip", "daddr"),
                    set_of(
                        NEVER_REACHABLE
                            .iter()
                            .map(|(address, length)| prefix(*address, u32::from(*length)))
                            .chain([prefix(policy.pool.network(), u32::from(policy.pool.prefix_len()))])
                            .collect(),
                    ),
                ),
                reject(),
            ],
        ),
        (
            "ingress",
            vec![
                Statement::Match(Match {
                    left: Expression::Named(NamedExpression::CT(CT {
                        key: "state".into(),
                        family: None,
                        dir: None,
                    })),
                    right: Expression::List(vec![text("established"), text("related")]),
                    op: Operator::IN,
                }),
                Statement::Accept(None),
            ],
        ),
    ];
    let denied = || {
        set_of(
            DENIED
                .iter()
                .map(|(address, length)| prefix(*address, *length))
                .collect(),
        )
    };
    let not_clouds = [
        ("ingress", vec![not_from_clouds(), matches(payload("ip", "daddr"), denied()), reject()]),
        ("ingress", vec![not_from_clouds(), Statement::Accept(None)]),
    ];
    let allowed = allowed_rules(policy.rules, denied());
    let closing = [
        ("ingress", vec![matches(payload("ip", "daddr"), text(&format!("@{NAMED}"))), Statement::Accept(None)]),
        (
            "ingress",
            vec![
                matches(meta(MetaKey::L4proto), set_of(vec![text("tcp"), text("udp")])),
                matches(
                    Expression::Named(NamedExpression::Concat(vec![payload("ip", "daddr"), payload("th", "dport")])),
                    text(&format!("@{NAMED_PORTS}")),
                ),
                Statement::Accept(None),
            ],
        ),
        ("ingress", vec![reject()]),
        (
            "nat",
            vec![
                matches(
                    payload("ip", "saddr"),
                    prefix(policy.pool.network(), u32::from(policy.pool.prefix_len())),
                ),
                differs(meta(MetaKey::Oifname), text(INTERFACES)),
                Statement::Masquerade(None),
            ],
        ),
    ];
    for (chain, statements) in rules.into_iter().chain(not_clouds).chain(allowed).chain(closing) {
        batch.add(NfListObject::Rule(Rule {
            family: NfFamily::INet,
            table: TABLE.into(),
            chain: chain.into(),
            expr: statements.into(),
            ..Rule::default()
        }));
    }
    batch.to_nftables()
}

/// The rules that accept what `rules` allows by address, which only a
/// Cloud's packets reach: the internet, every public destination, as one
/// rule, then one for the networks on every port and one for each port the
/// others name.
fn allowed_rules(rules: &NetworkRules, denied: Expression<'static>) -> Vec<(&'static str, Vec<Statement<'static>>)> {
    let internet = rules
        .internet
        .then(|| ("ingress", vec![differs(payload("ip", "daddr"), denied), Statement::Accept(None)]));
    let mut by_port: BTreeMap<Option<u16>, Vec<Ipv4Net>> = BTreeMap::new();
    if rules.private_network {
        by_port.entry(None).or_default().extend(
            PRIVATE_NETWORKS
                .iter()
                .map(|(address, length)| Ipv4Net::new(*address, *length).expect("a valid prefix length")),
        );
    }
    for allowed in &rules.allowed {
        if let Allowed::Network(destination) = allowed {
            by_port.entry(destination.port).or_default().push(destination.network);
        }
    }
    let by_address = by_port.into_iter().map(|(port, networks)| {
        let mut statements = vec![matches(
            payload("ip", "daddr"),
            set_of(
                networks
                    .iter()
                    .map(|network| prefix(network.network(), u32::from(network.prefix_len())))
                    .collect(),
            ),
        )];
        if let Some(port) = port {
            statements.push(matches(meta(MetaKey::L4proto), set_of(vec![text("tcp"), text("udp")])));
            statements.push(matches(payload("th", "dport"), Expression::Number(u32::from(port))));
        }
        statements.push(Statement::Accept(None));
        ("ingress", statements)
    });
    internet.into_iter().chain(by_address).collect()
}

/// Opens `named` for their timeouts, or for longer ones: an element that is
/// in its set is replaced, since adding an element that exists keeps its
/// old timeout. Adding each first makes its deletion succeed either way.
pub fn add_named(named: &[Named]) -> Nftables<'static> {
    let mut batch = Batch::new();
    for element in named_elements(named) {
        let bare = Element {
            elem: element
                .elem
                .iter()
                .map(|expression| match expression {
                    Expression::Named(NamedExpression::Elem(elem)) => (*elem.val).clone(),
                    other => other.clone(),
                })
                .collect(),
            ..element.clone()
        };
        batch.add(NfListObject::Element(bare.clone()));
        batch.delete(NfListObject::Element(bare));
        batch.add(NfListObject::Element(element));
    }
    batch.to_nftables()
}

/// Each of `named` as an element of its set, with its timeout.
fn named_elements(named: &[Named]) -> Vec<Element<'static>> {
    named
        .iter()
        .map(|named| {
            let address = text(&named.address.to_string());
            let (set, value) = match named.port {
                None => (NAMED, address),
                Some(port) => (
                    NAMED_PORTS,
                    Expression::Named(NamedExpression::Concat(vec![address, Expression::Number(u32::from(port))])),
                ),
            };
            Element {
                family: NfFamily::INet,
                table: TABLE.into(),
                name: set.into(),
                elem: vec![Expression::Named(NamedExpression::Elem(Elem {
                    val: Box::new(value),
                    timeout: Some(named.timeout),
                    expires: None,
                    comment: None,
                    counter: None,
                }))]
                .into(),
            }
        })
        .collect()
}

/// A set of `kind` whose elements expire.
fn named_set(name: &'static str, kind: SetTypeValue<'static>) -> NfListObject<'static> {
    NfListObject::Set(Box::new(Set {
        family: NfFamily::INet,
        table: TABLE.into(),
        name: name.into(),
        handle: None,
        set_type: kind,
        policy: None,
        flags: Some(HashSet::from([SetFlag::Timeout])),
        elem: None,
        timeout: None,
        gc_interval: None,
        size: None,
        comment: None,
    }))
}

/// The sets a sandbox of `machine` is admitted to.
fn sets_of(machine: SandboxMachine) -> &'static [&'static str] {
    match machine {
        SandboxMachine::Cloud => &[SLOTS, CLOUDS],
        SandboxMachine::ClaudeCode => &[SLOTS],
    }
}

/// Admits `slot`'s pair before its sandbox, a `machine`, runs.
pub fn add_slot(slot: &Slot, machine: SandboxMachine) -> Nftables<'static> {
    let mut batch = Batch::new();
    for name in sets_of(machine) {
        batch.add(NfListObject::Element(element(name, slot)));
    }
    batch.to_nftables()
}

/// Removes `slot`'s pair from each set that has it. Adding the table, the
/// sets and the elements first makes the deletion succeed whether or not
/// they existed: after a host reboot, or with a table an earlier manager
/// left, there is nothing to remove, and the next reconciliation replaces
/// the table.
pub fn remove_slot(slot: &Slot) -> Nftables<'static> {
    let mut batch = Batch::new();
    batch.add(NfListObject::Table(table_object()));
    for name in [SLOTS, CLOUDS] {
        batch.add(set(name));
        batch.add(NfListObject::Element(element(name, slot)));
        batch.delete(NfListObject::Element(element(name, slot)));
    }
    batch.to_nftables()
}

fn table_object() -> Table<'static> {
    Table {
        family: NfFamily::INet,
        name: TABLE.into(),
        handle: None,
    }
}

fn set(name: &'static str) -> NfListObject<'static> {
    NfListObject::Set(Box::new(Set {
        family: NfFamily::INet,
        table: TABLE.into(),
        name: name.into(),
        handle: None,
        set_type: SetTypeValue::Concatenated(Cow::Owned(vec![SetType::Ifname, SetType::Ipv4Addr])),
        policy: None,
        flags: None,
        elem: None,
        timeout: None,
        gc_interval: None,
        size: None,
        comment: None,
    }))
}

fn element(name: &'static str, slot: &Slot) -> Element<'static> {
    Element {
        family: NfFamily::INet,
        table: TABLE.into(),
        name: name.into(),
        elem: vec![Expression::Named(NamedExpression::Concat(vec![
            text(&slot.host_interface()),
            text(&slot.address.to_string()),
        ]))]
        .into(),
    }
}

fn base_chain(name: &str, kind: NfChainType, hook: NfHook, priority: i32) -> Chain<'static> {
    Chain {
        family: NfFamily::INet,
        table: TABLE.into(),
        name: name.to_owned().into(),
        _type: Some(kind),
        hook: Some(hook),
        prio: Some(priority),
        policy: Some(NfChainPolicy::Accept),
        ..Chain::default()
    }
}

fn regular_chain(name: &str) -> Chain<'static> {
    Chain {
        family: NfFamily::INet,
        table: TABLE.into(),
        name: name.to_owned().into(),
        ..Chain::default()
    }
}

/// Refuses the packet at once: "No route to host" to its sender.
fn reject() -> Statement<'static> {
    Statement::Reject(Some(Reject::new(Some(RejectType::ICMPX), Some(RejectCode::AdminProhibited))))
}

fn text(value: &str) -> Expression<'static> {
    Expression::String(value.to_owned().into())
}

fn meta(key: MetaKey) -> Expression<'static> {
    Expression::Named(NamedExpression::Meta(Meta { key }))
}

fn payload(protocol: &'static str, field: &'static str) -> Expression<'static> {
    Expression::Named(NamedExpression::Payload(Payload::PayloadField(
        PayloadField {
            protocol: protocol.into(),
            field: field.into(),
        },
    )))
}

fn prefix(address: Ipv4Addr, length: u32) -> Expression<'static> {
    Expression::Named(NamedExpression::Prefix(Prefix {
        addr: Box::new(text(&address.to_string())),
        len: length,
    }))
}

fn set_of(items: Vec<Expression<'static>>) -> Expression<'static> {
    Expression::Named(NamedExpression::Set(
        items.into_iter().map(SetItem::Element).collect(),
    ))
}

fn addresses(addresses: &[Ipv4Addr]) -> Expression<'static> {
    set_of(
        addresses
            .iter()
            .map(|address| text(&address.to_string()))
            .collect(),
    )
}

fn matches(left: Expression<'static>, right: Expression<'static>) -> Statement<'static> {
    Statement::Match(Match {
        left,
        right,
        op: Operator::EQ,
    })
}

fn differs(left: Expression<'static>, right: Expression<'static>) -> Statement<'static> {
    Statement::Match(Match {
        left,
        right,
        op: Operator::NEQ,
    })
}
