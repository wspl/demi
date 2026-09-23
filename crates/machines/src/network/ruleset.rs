//! The firewall (`managed-hosts.md` § Networking): one fixed nftables table
//! whose rules are the same for every sandbox. Its only per-sandbox fact is
//! an element of the `slots` set, the pair of a sandbox's host interface and
//! source address, so a packet from a Cloud interface with any other pair is
//! dropped.
//!
//! ```text
//! table inet demi_cloud {
//!   set slots { type ifname . ipv4_addr; }
//!   chain input   { filter hook input -10; iifname "demih*" jump ingress }
//!   chain forward { filter hook forward -10; iifname "demih*" jump ingress;
//!                   oifname "demih*" ct state established,related accept; oifname "demih*" drop }
//!   chain ingress { meta nfproto != ipv4 drop; iifname . ip saddr != @slots drop;
//!                   ip daddr {backend} tcp dport <port> accept;
//!                   ip daddr {dns} meta l4proto {tcp, udp} th dport 53 accept;
//!                   fib daddr type local drop; ip daddr {denied} drop; accept }
//!   chain nat     { nat hook postrouting srcnat; ip saddr <pool> oifname != "demih*" masquerade }
//! }
//! ```

use std::{borrow::Cow, collections::HashSet, net::Ipv4Addr};

use ipnet::Ipv4Net;
use nftables::{
    batch::Batch,
    expr::{Expression, Fib, FibFlag, FibResult, Meta, MetaKey, NamedExpression, Payload, PayloadField, Prefix, SetItem, CT},
    schema::{Chain, Element, NfListObject, Nftables, Rule, Set, SetType, SetTypeValue, Table},
    stmt::{JumpTarget, Match, Operator, Statement},
    types::{NfChainPolicy, NfChainType, NfFamily, NfHook},
};

use super::slots::Slot;

pub const TABLE: &str = "demi_cloud";
const SET: &str = "slots";
/// Every Cloud host interface's name starts with this.
const INTERFACES: &str = "demih*";
/// The filter chains run just before the default priority.
const FILTER_PRIORITY: i32 = -10;
/// `srcnat`: the priority of source NAT in the postrouting hook.
const SOURCE_NAT_PRIORITY: i32 = 100;

/// Destinations a sandbox may not reach unless an exact rule before this
/// one allows it: private, shared, loopback, link-local (metadata),
/// documentation, benchmarking, multicast and reserved ranges.
const DENIED: [(Ipv4Addr, u32); 14] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

/// The allowed endpoints besides public destinations.
pub struct Policy<'a> {
    pub pool: Ipv4Net,
    pub backend: &'a [Ipv4Addr],
    pub backend_port: u16,
    pub dns: &'a [Ipv4Addr],
}

/// The whole table, replacing any table of that name in one transaction:
/// adding it first makes the deletion succeed whether or not it existed.
pub fn table(policy: &Policy<'_>) -> Nftables<'static> {
    let mut batch = Batch::new();
    batch.add(NfListObject::Table(table_object()));
    batch.delete(NfListObject::Table(table_object()));
    batch.add(NfListObject::Table(table_object()));
    batch.add(set());
    let chains = [
        base_chain("input", NfChainType::Filter, NfHook::Input, FILTER_PRIORITY),
        base_chain("forward", NfChainType::Filter, NfHook::Forward, FILTER_PRIORITY),
        regular_chain("ingress"),
        base_chain("nat", NfChainType::NAT, NfHook::Postrouting, SOURCE_NAT_PRIORITY),
    ];
    for chain in chains {
        batch.add(NfListObject::Chain(chain));
    }
    let jump = || Statement::Jump(JumpTarget { target: "ingress".into() });
    let rules = [
        ("input", vec![matches(meta(MetaKey::Iifname), text(INTERFACES)), jump()]),
        ("forward", vec![matches(meta(MetaKey::Iifname), text(INTERFACES)), jump()]),
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
        ("forward", vec![matches(meta(MetaKey::Oifname), text(INTERFACES)), Statement::Drop(None)]),
        ("ingress", vec![differs(meta(MetaKey::Nfproto), text("ipv4")), Statement::Drop(None)]),
        (
            "ingress",
            vec![
                differs(
                    Expression::Named(NamedExpression::Concat(vec![meta(MetaKey::Iifname), payload("ip", "saddr")])),
                    text(&format!("@{SET}")),
                ),
                Statement::Drop(None),
            ],
        ),
        (
            "ingress",
            vec![
                matches(payload("ip", "daddr"), addresses(policy.backend)),
                matches(payload("tcp", "dport"), Expression::Number(u32::from(policy.backend_port))),
                Statement::Accept(None),
            ],
        ),
        (
            "ingress",
            vec![
                matches(payload("ip", "daddr"), addresses(policy.dns)),
                matches(meta(MetaKey::L4proto), set_of(vec![text("tcp"), text("udp")])),
                matches(payload("th", "dport"), Expression::Number(53)),
                Statement::Accept(None),
            ],
        ),
        (
            "ingress",
            vec![
                matches(
                    Expression::Named(NamedExpression::Fib(Fib {
                        result: FibResult::Type,
                        flags: HashSet::from([FibFlag::Daddr]),
                    })),
                    text("local"),
                ),
                Statement::Drop(None),
            ],
        ),
        (
            "ingress",
            vec![
                matches(
                    payload("ip", "daddr"),
                    set_of(DENIED.iter().map(|(address, length)| prefix(*address, *length)).collect()),
                ),
                Statement::Drop(None),
            ],
        ),
        ("ingress", vec![Statement::Accept(None)]),
        (
            "nat",
            vec![
                matches(payload("ip", "saddr"), prefix(policy.pool.network(), u32::from(policy.pool.prefix_len()))),
                differs(meta(MetaKey::Oifname), text(INTERFACES)),
                Statement::Masquerade(None),
            ],
        ),
    ];
    for (chain, statements) in rules {
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

/// Admits `slot`'s pair before its sandbox runs.
pub fn add_slot(slot: &Slot) -> Nftables<'static> {
    let mut batch = Batch::new();
    batch.add(NfListObject::Element(element(slot)));
    batch.to_nftables()
}

/// Removes `slot`'s pair if the table has it. Adding the table, the set and
/// the element first makes the deletion succeed whether or not they existed:
/// after a host reboot, or with a table an earlier manager left, there is
/// nothing to remove, and the next reconciliation replaces the table.
pub fn remove_slot(slot: &Slot) -> Nftables<'static> {
    let mut batch = Batch::new();
    batch.add(NfListObject::Table(table_object()));
    batch.add(set());
    batch.add(NfListObject::Element(element(slot)));
    batch.delete(NfListObject::Element(element(slot)));
    batch.to_nftables()
}

fn table_object() -> Table<'static> {
    Table {
        family: NfFamily::INet,
        name: TABLE.into(),
        handle: None,
    }
}

fn set() -> NfListObject<'static> {
    NfListObject::Set(Box::new(Set {
        family: NfFamily::INet,
        table: TABLE.into(),
        name: SET.into(),
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

fn element(slot: &Slot) -> Element<'static> {
    Element {
        family: NfFamily::INet,
        table: TABLE.into(),
        name: SET.into(),
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

fn text(value: &str) -> Expression<'static> {
    Expression::String(value.to_owned().into())
}

fn meta(key: MetaKey) -> Expression<'static> {
    Expression::Named(NamedExpression::Meta(Meta { key }))
}

fn payload(protocol: &'static str, field: &'static str) -> Expression<'static> {
    Expression::Named(NamedExpression::Payload(Payload::PayloadField(PayloadField {
        protocol: protocol.into(),
        field: field.into(),
    })))
}

fn prefix(address: Ipv4Addr, length: u32) -> Expression<'static> {
    Expression::Named(NamedExpression::Prefix(Prefix {
        addr: Box::new(text(&address.to_string())),
        len: length,
    }))
}

fn set_of(items: Vec<Expression<'static>>) -> Expression<'static> {
    Expression::Named(NamedExpression::Set(items.into_iter().map(SetItem::Element).collect()))
}

fn addresses(addresses: &[Ipv4Addr]) -> Expression<'static> {
    set_of(addresses.iter().map(|address| text(&address.to_string())).collect())
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
