---
status: proposed
---

# Clouds can reach the private network when the master allows it

## Problem

A company runs Demi on its own network, with its Git server at
`git.example.internal`, `10.20.0.7`. An agent in a Cloud runs
`git clone git@git.example.internal:team/app.git`. The name resolves, but the
firewall drops every packet to a private range, so the clone hangs until it
times out and says nothing about why. An internal HTTP API and the company
proxy fail the same way. Nobody can allow them: the rules are fixed in the
manager, which replaces its table on every reconcile, and the agent cannot
tell the user what is wrong or where to change it.

## Decision

The master chooses whether Clouds reach the private network:

```text
Settings › Cloud Network

  Private Network Access     [ Off                       ▾ ]
                               Off
                               Entire Private Network
                               Only These Addresses

  Clouds reach only the internet by default. If Demi runs on
  your company's network, turn this on so Clouds can reach its
  Git servers, internal services and proxies. The server itself,
  cloud metadata services and other people's Clouds stay
  unreachable.
```

With Only These Addresses, a group of addresses follows: one bare field
whose placeholder reads "Add an address, such as 10.20.0.0/16 or
10.20.0.7:22", then the addresses as plain rows, as a provider's models are
under their filter. Enter adds what the field holds, and each row has a
remove button. An address is changed by removing it and adding it again, as
an entry of macOS's list of DNS servers is. What the server refuses stays in
the field, with why under it.

- **Off**, the default, refuses what today's policy does.
- **Entire Private Network** allows `10.0.0.0/8`, `172.16.0.0/12`,
  `192.168.0.0/16` and `100.64.0.0/10`, the shared range Tailscale's
  addresses are in. The page adds that a Cloud is then on the network as an
  employee's computer is: a web page its browser opens can try to reach an
  internal service too.
- **Only These Addresses** allows each entry: an IPv4 address or network,
  with an optional `:port` that allows that TCP or UDP port only, every port
  and protocol without one. A row that does not parse, or touches a range no
  Cloud may reach, shows why under it and is not saved.
- **What no choice opens**: the server's own addresses (`fib daddr type
  local`), the sandbox pool, which is the other Clouds, loopback,
  `0.0.0.0/8`, link-local `169.254.0.0/16`, where metadata services answer,
  multicast and reserved ranges.
- **Who.** Only the master sees the section and changes it, as only the
  master sets the instance mode. It applies to every Cloud on the server, a
  person's and an integration's, and never to the Claude Code machine,
  which reaches only its vendor.
- **Names** are not entries. A Cloud resolves an internal name through the
  resolvers `DEMI_MANAGED_DNS` names, and the firewall checks the address the
  connection goes to, so a public name that resolves to a private address
  still opens nothing the setting did not.

### A blocked connection says so

Every denied destination is rejected instead of dropped, with ICMP
administratively prohibited for every protocol. A command then fails at once,
`ssh` and `git clone` with "No route to host", instead of waiting for a
timeout. A TCP reset would say "Connection refused", which reads as a server
that is down rather than a network that is closed.

The context block's primary host line of a Cloud gains one sentence, so an
agent that meets the error tells the user where to change it: "Addresses on
the server's private network are blocked unless the server's master allows
them in Settings › Cloud Network." It holds whatever the setting is, so a
change of the setting does not make an earlier block wrong.

### The table

```text
chain ingress {
  meta nfproto != ipv4 drop; iifname . ip saddr != @slots drop
  ip daddr {dns} meta l4proto {tcp, udp} th dport 53 accept
  fib daddr type local reject
  ip daddr {never reachable, pool} reject
  iifname . ip saddr @clouds ip daddr 10.20.0.0/16 accept
  iifname . ip saddr @clouds ip daddr 10.20.0.7 meta l4proto {tcp, udp}
      th dport 22 accept
  ip daddr {denied} reject; accept
}
```

`clouds` holds the pair of each Cloud sandbox, the subset of `slots` that is
not the Claude Code machine. The setting gives one accepting rule for each
range or entry, none when it is off. The server's own addresses, the ranges
no Cloud reaches and the pool are rejected before them, so no entry opens
them.

### Backend and manager

```rust
// machine-manager-protocol
pub enum PrivateNetwork {
    Off,
    Entire,
    Only(Vec<Destination>),
}
pub struct Destination {
    pub network: Ipv4Net,
    pub port: Option<u16>,
}
pub struct SetPrivateNetworkParams { pub private_network: PrivateNetwork }
```

- The backend stores the choice and the list as entered in `server_settings`
  and parses each entry into a `Destination`, so the page shows what the
  master typed and the manager gets what the backend checked. The backend
  checks every entry whatever the choice, so a stored list always reads.
- `set_private_network` replaces the setting and installs the table with
  every running sandbox's pairs, as reconcile does. The manager keeps it in
  memory only and starts with Off, so the backend sends it each time it
  connects to the manager, as well as after each change: a manager that
  restarted on its own gets it back as soon as the backend reconnects.
- `PUT /settings/cloud-network`, which only the master sends, stores the
  setting and then sends `set_private_network`. When the manager cannot be
  reached, the setting stays stored, the request fails, and the page says
  that Clouds get it when the manager is back. When the manager refuses it,
  the request fails with the manager's reason.
- The wire version goes up: the backend and the manager run one release.

### Proxies

A Cloud uses an HTTP proxy as any Linux machine does: an agent or a person
exports `HTTPS_PROXY` in the Cloud's `~/.profile`, which every command reads,
since a job's shell is a login shell. The setting only needs to allow the
proxy's address. Demi adds no proxy setting.

## Spike

- **A job reads `~/.profile`.** A file holding one `export` was written to a
  Host's home, and the next command of a conversation there printed its
  value. Every Host's jobs run the same shell, a Cloud's included.
- **Each choice, in the Lima VM** of
  [Develop on a Mac with Lima](../guides/mac-development.md). The manager
  took each choice through `set_private_network`. A network namespace on a
  slot's address, added to the table's sets as the manager adds a Cloud's,
  connected to HTTP and SSH stand-ins on a second namespace at `10.20.0.7`
  and `10.20.0.8`:
  - Off refused both, and the public internet answered.
  - Entire Private Network reached both. The VM's own addresses and
    `169.254.169.254` stayed refused.
  - Only These Addresses with `10.20.0.7:8080` and `10.20.0.8` reached port
    8080 of the first and every port of the second, and refused port 2222
    of the first.
- **A refusal is quick.** `curl` failed in under a millisecond, and `ssh`
  printed "No route to host". The kernel sends at most six ICMP errors at
  once to one address, then one a second (`net.ipv4.icmp_ratelimit`), so a
  Cloud that keeps connecting to refused addresses waits about a second for
  each after the sixth. A TCP reset has no such limit, but its words mislead.
  A Cloud's gVisor stack ends a connection being opened on this ICMP error
  with "No route to host" (`pkg/tcpip/transport/tcp/endpoint.go`,
  `onICMPError`); that was read in its code, not run.

## Prior art

- **macOS** asks once whether an app may reach the local network, an on or
  off choice a person understands, and keeps the addresses out of it. Demi's
  first two choices are that switch.
- **Anthropic's sandbox-runtime** keeps a list of allowed destinations, with
  `host:port` entries; Demi's Only These Addresses takes its `:port` form.
  sandbox-runtime also takes names, through an HTTP and a SOCKS proxy every
  connection goes through; Demi leaves names out, since a Cloud is a whole
  machine whose programs, `ssh` among them, do not all take a proxy.
- **Cilium** allows names by watching the answers of a DNS proxy
  ([DNS-based policies](https://docs.cilium.io/en/stable/security/dns/)).
  That is how Demi would add names later, if Only These Addresses turns out
  to need them.

## Alternatives

- **Only a list of destinations.** It asks for networks and ports from a
  person who wants the Git server to work, and most companies want the whole
  network anyway.
- **Names in the list now.** The manager would answer the Clouds' DNS itself,
  so that only an answer for an allowed name opens its address; Entire
  Private Network covers most cases without it.
- **An environment variable on the manager, as `DEMI_MANAGED_DNS` is.** The
  master would need the host and a restart of the manager, and the page
  could not show the choice.
- **A proxy setting in Demi.** `~/.profile` already does it, per Cloud.
- **Each address an editable field.** A list of text fields, each saved
  when its edit ends, reads as a form inside the card and doubles the
  states a row has, for an address that is short to type again.

## Open questions

None.
