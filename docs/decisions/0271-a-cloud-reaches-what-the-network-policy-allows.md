---
status: proposed
---

# A Cloud reaches what the server's network policy allows

Supersedes: 0213 (Clouds can reach the private network when the master
allows it), its setting: the three choices of Private Network Access, a list
of addresses alone, and names left out of it. What a denied connection says,
the ranges no choice opens, who changes the setting and how a Cloud uses a
proxy still hold.

## Problem

Alex runs Demi on a server at home and wants the agents' Clouds to install
packages from npm and PyPI and push to GitHub, and to reach nothing else, so
that a prompt hidden in a web page an agent reads cannot have it fetch more
instructions from, or post the code to, any site at all. Demi cannot do that:
the internet is always open to a Cloud,
and the only setting, Private Network Access, decides what a Cloud reaches on
the server's private network. Its description speaks to a company's network
("If Demi runs on your company's network, turn this on…"), which reads as a
setting for one kind of server, not as what a Cloud may reach.

## Decision

Private Network Access becomes Network Policy, which decides every
destination a Cloud reaches: one menu of common policies, and Custom, which
allows exactly what the master lists.

```text
Settings › Cloud
┌──────────────────────────────────────────────────────────────────┐
│ Isolation                                         [ gVisor ▾ ]   │
├──────────────────────────────────────────────────────────────────┤
│ Network Policy                             [ Internet Only ▾ ]   │
│ Clouds reach the internet, and nothing on the server's network.  │
└──────────────────────────────────────────────────────────────────┘
                                                 No Network
                                                 Developer Services
                                               ✓ Internet Only
                                                 Internet and Private Network
                                                 ─────────────
                                                 Custom
```

| Policy | A Cloud reaches |
| --- | --- |
| No Network | Nothing |
| Developer Services | Git hosts, package registries and the downloads of build tools, such as `github.com`, `npmjs.org` and `pypi.org` |
| Internet Only, the default | Every public address |
| Internet and Private Network | Every public address and the private ranges 0213's Entire Private Network opens |
| Custom | What the master allows below |

Custom shows what it allows under the menu:

```text
Allowed
┌──────────────────────────────────────────────────────────────────┐
│ Internet                                                  [ ● ]  │
│ Every public address.                                            │
│ Private Network                                           [   ]  │
│ 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 and 100.64.0.0/10.     │
│ Developer Services                                        [   ]  │
│ GitHub, npm, PyPI and 68 more.                                   │
└──────────────────────────────────────────────────────────────────┘
Domains and Addresses
Clouds reach each one: on every port, or only on the port after it.
┌──────────────────────────────────────────────────────────────────┐
│ + Add domains or addresses, such as *.example.com or 10.20.0.7:22│
│ git.example.com                                              🗑   │
│ 10.20.0.0/16                                                 🗑   │
└──────────────────────────────────────────────────────────────────┘
```

- **Choosing Custom** sets the three switches to the policy chosen before,
  so the master starts from it, as Codex starts a custom list from a preset.
  The domains and addresses stay listed while another policy is chosen, as
  0213's list did, and apply only under Custom.
- **An entry** is an IPv4 address or network, as in 0213, or a domain: an
  exact name, `git.example.com`, or every name under one but not the name
  itself, `*.example.com`. Either takes `:port`, which allows that TCP or
  UDP port only. Developer Services' domains each allow the name and every
  name under it, since a registry serves from its subdomains.
- **A listed domain opens what it resolves to**, a private address included:
  `git.example.internal` at `10.20.0.7` is reachable without listing the
  address. This replaces 0213's rule that a name opens nothing; a name
  opens only what its entry allows, and only the master writes entries.
- **Developer Services** is a list Demi ships and each release may change,
  starting from the one Codex publishes for the same purpose. It is what
  builds download from, not a guard against data leaving: GitHub, which it
  allows, accepts pushes, and some of its domains, such as `google.com`,
  serve documents too. Its description says so in plain words: "the services
  builds download from".
- **While the internet is off**, a name resolves only if an entry or
  Developer Services lists it, a name on the private network included; the
  list's description says so.
- **The internet** is every address but the private, shared, loopback,
  link-local, documentation, multicast and reserved ranges. The benchmarking
  range, `198.18.0.0/15`, counts as the internet: a proxy in fake-IP mode,
  such as Clash's, answers every name with an address there and carries the
  connection to the real site, so a server whose resolver is such a proxy
  would otherwise reach nothing, as happened in the spike's VM.
- **What no policy opens** is 0213's: the server's own addresses, the other
  Clouds, loopback, link-local where metadata services answer, multicast and
  reserved ranges. A domain whose answer is one of them opens nothing.
- **A change** applies to new connections; a connection already open
  finishes, as Cilium's policies do.
- **Simple Isolation**, where Clouds share the server's network (0268), keeps
  Network Policy visible and disabled, with that reason, as it keeps Private
  Network Access today.
- **Internet and Private Network** keeps 0213's warning in its description: a
  web page a Cloud's browser opens can try to reach internal services too.
- **What the agent reads**: the sentence in a Cloud's context becomes "The
  server's network policy, which its master sets in Settings › Cloud,
  decides what this Cloud reaches: a destination it does not allow fails
  with "No route to host", and a name it does not allow may not resolve."

### How a domain is allowed

The firewall sees addresses, not names, so the manager answers each Cloud's
DNS itself and opens the addresses of the names the policy allows, as
Cilium's DNS proxy and dnsmasq's `nftset` do:

```text
Cloud ── DNS to any address ──> redirected to the manager on the Cloud's
                                gateway:53 ──> DEMI_MANAGED_DNS
           │ a name the policy allows: before answering, adds each address
           │ of the answer to the set `named` (or `named_ports` with its
           ▼ port)
       nftables ── ip daddr @named accept ──> the destination
```

- Every question a Cloud sends on TCP or UDP port 53, to whatever address,
  is redirected to its slot's gateway, the host end of its pair, where the
  manager answers on a port of its own, 25353: a resolver of the host's,
  such as dnsmasq on every address, may hold port 53, which the manager
  could then not listen on. A Cloud's `/etc/resolv.conf` keeps naming the
  resolvers `DEMI_MANAGED_DNS` names, so a Cloud that was running before the upgrade
  goes on resolving, and none can go around the policy by asking another
  resolver. The manager forwards each question it does not refuse to those
  resolvers.
- An address of an allowed name is added before the answer is sent, so the
  connection that follows the answer is allowed. It stays in the set for the
  answer's TTL, at least five minutes and at most a day, since programs reuse
  an address after its TTL and the JVM caches one for 30 seconds whatever it
  says; each new answer renews it.
- A question for a name the policy does not list is answered as usual when
  the policy allows the internet. Otherwise it is refused (`REFUSED`), so a
  Cloud cannot carry data out in the names it asks about, and a command says
  it could not resolve the host. A message of more than one question gets a
  format error, since an upstream resolver might answer the second for a name
  that was not allowed, and so does one that is no DNS message, so the
  client fails at once.
- One lock orders every change of the sets and the table. An answer that
  was asked upstream while the rules changed is decided again under it, by
  the new rules, before it opens anything.
- The Claude Code machine keeps asking `DEMI_MANAGED_DNS` itself and
  reaching the internet, whatever the policy.

### The table

```text
chain resolver {
  type nat hook prerouting priority dstnat
  iifname . ip saddr @clouds meta l4proto {tcp, udp} th dport 53
      redirect to :25353
}
chain ingress {
  meta nfproto != ipv4 drop; iifname . ip saddr != @slots drop
  iifname . ip saddr @clouds meta l4proto {tcp, udp} th dport 25353
      fib daddr type local accept
  ip daddr {dns} meta l4proto {tcp, udp} th dport 53 accept
  fib daddr type local reject
  ip daddr {never reachable, pool} reject
  ct state established,related accept
  iifname . ip saddr != @clouds ip daddr {denied} reject
  iifname . ip saddr != @clouds accept
  <one rule for each group or address the policy allows>
  ip daddr @named accept
  meta l4proto {tcp, udp} ip daddr . th dport @named_ports accept
  reject
}
```

Internet is one rule, `ip daddr != {denied} accept`; Private Network one
rule for its four ranges; each address one rule, as in 0213. `named` and
`named_ports` are sets whose elements time out. The manager keeps what it
added, so a table it replaces with the same rules, as a reconcile does, gets
them back with the time they have left; a change of the rules starts them
empty, and the next answer for each allowed name adds it again.

### Backend and manager

```rust
// machine-manager-protocol: what the manager applies
pub struct NetworkRules {
    pub internet: bool,
    pub private_network: bool,
    pub developer_services: bool,
    pub allowed: Vec<Allowed>,
}
pub enum Allowed {
    Network(Destination),             // 0213's
    Domain(Domain),
}
pub struct Domain { pub name: DomainName, pub port: Option<u16> }
pub enum DomainName {
    Exact(String),                    // git.example.com
    Under(String),                    // *.example.com
}
pub const DEVELOPER_SERVICES: &[&str] = &["github.com", /* … */];
pub struct SetNetworkRulesParams { pub network_rules: NetworkRules }

// web-api-protocol, GET and PUT /settings/cloud-network
pub enum NetworkPolicy {
    NoNetwork, DeveloperServices, InternetOnly,
    InternetAndPrivateNetwork, Custom,
}
pub struct CustomNetworkPolicy {
    pub internet: bool,
    pub private_network: bool,
    pub developer_services: bool,
    pub allowed: Vec<String>,         // as the master wrote them
}
pub struct CloudNetwork {             // GET
    pub policy: NetworkPolicy,
    pub custom: CustomNetworkPolicy,
    /// The domains of Developer Services, for the page's count.
    pub developer_services: Vec<String>,
}
pub struct SetCloudNetwork {          // PUT
    pub policy: NetworkPolicy,
    pub custom: CustomNetworkPolicy,
}
```

- The backend turns the policy into `NetworkRules` and sends them as it sends
  Private Network today: on each connection and after each change.
- One concept keeps one name: `PrivateNetwork`, `SetPrivateNetworkParams`,
  0268's `Enforces::private_network` and the page's keys are renamed to the
  network policy and its rules, and the wire version goes up.
- A migration turns `server_settings`' `private_network` and
  `private_destinations` into `network_policy` and the `custom_*` columns:
  Off becomes Internet Only, Entire Private Network becomes Internet and
  Private Network, and Only These Addresses becomes Custom with Internet on
  and the same addresses, so every Cloud reaches what it reached before.

## Spike

The points the design was not sure of are whether answering DNS and filling a
set with timeouts lets the first connection through and keeps a long one
alive, and whether the redirect sends a Cloud's every question to the
manager. Two runs settled them.

**dnsmasq's `nftset`**, as root in a throwaway network namespace on Linux
6.18: a "Cloud" namespace behind a veth pair, dnsmasq on the gateway filling
a set whose elements expired after six seconds, and a table like the one
above.

- The first lookup of `example.com`, and of `api.github.com` under the
  listed `github.com`, connected at once: the address was in the set before
  the answer arrived.
- `example.org`, not listed, resolved and was refused in under a
  millisecond, as was `1.1.1.1`, which no answer named.
- A download from a listed host ran for 25 seconds, past the six-second
  timeout, and stopped only at curl's limit: `ct state established` keeps an
  open connection.
- A new connection to `example.com`'s address seven seconds after its
  lookup, without a new one, was refused: the reason for the five-minute
  floor.

**The manager's own network code**, in the Lima VM of
[Develop on a Mac with Lima](../guides/mac-development.md), Linux 7.0: a
program drove `CloudNetwork` with a Cloud's slot and the Claude Code
machine's, and ran `curl`, `dig` and `nc` in their namespaces. A second
namespace at `10.77.0.2` served HTTP and port 2222, and a stand-in for a
company's resolver answered `internal.example.test` with that address.

- Internet Only reached `example.com` and refused `10.77.0.2`.
- No Network refused `1.1.1.1` and answered `example.com` with `REFUSED`,
  also when `dig` asked `8.8.8.8`: the redirect reached the manager. The
  Claude Code machine still reached `example.com`.
- Developer Services reached `github.com`, `api.github.com` and
  `registry.npmjs.org`, and `example.com` did not resolve. The set held
  their addresses for five minutes.
- Custom with `example.com:443`, `*.example.org`, `internal.example.test`
  and `10.77.0.2:8080` reached `https://example.com` but not port 80,
  `www.example.org` but not `example.org`, both ports of
  `internal.example.test` at its private address, and the listed address;
  `github.com` did not resolve.
- Internet and Private Network reached `example.com` and both ports of
  `10.77.0.2`, and refused the gateway and `169.254.169.254`.
- A table installed again with the same rules kept `example.com`'s two
  addresses with the time they had left; other rules emptied the set.
- A download from a listed host went on for ten seconds after the rules
  changed to No Network, until curl's limit; a new lookup was refused.
- Once the slot was removed, no listener was left on its gateway.

## Prior art

- **Claude Code on the web** offers None, Trusted (package registries,
  GitHub, cloud SDKs), Full and Custom, whose list of domains can include
  Trusted's
  ([cloud environments](https://code.claude.com/docs/en/cloud-environments.md)).
  Demi's menu is the same, and splits Full in two, since a Cloud runs on the
  master's own network, which Anthropic's sandboxes do not.
- **Codex cloud** turns access on or off and narrows it with a domain list
  that starts from None, Common dependencies or All
  ([agent internet access](https://developers.openai.com/codex/cloud/agent-internet)).
  Developer Services starts from its Common dependencies. Codex can also
  allow only GET, HEAD and OPTIONS; Demi does not, since that needs a proxy
  that reads every request.
- **Cilium's DNS proxy** answers a pod's DNS, refuses the names a policy
  does not allow, and opens the addresses of those it does
  ([`pkg/fqdn`](https://github.com/cilium/cilium/blob/main/pkg/fqdn/doc.go)).
  Demi does the same in the manager. Cilium keeps no floor on the TTL by
  default, because its floor cost memory across a cluster's pods; a server's
  Clouds are few, so Demi keeps one.
- **dnsmasq's `nftset`** adds the addresses of the domains it lists to an
  nftables set as it answers, which the spike used.

## Alternatives

- **A proxy for every connection**, as Claude Code and Anthropic's
  sandbox-runtime use. It sees the name of each HTTPS request, so two sites
  on one address stay apart, but a Cloud is a whole machine whose programs,
  `ssh` and `git` over SSH among them, do not all take a proxy.
- **Names checked by their answers alone**, without refusing the others.
  With the internet closed, a Cloud could still carry data out in the names
  it asks a resolver about.
- **Addresses only, without domains.** Developer Services would be
  impossible, since registries' addresses change, and No Network would be the
  only closed policy.
- **A policy per user or per conversation.** Codex and Claude Code set one
  per environment. A server has one network and one master, so one policy
  covers what 0213's setting covered; a choice per Cloud can come later on
  top of it.

## Open questions

None.
