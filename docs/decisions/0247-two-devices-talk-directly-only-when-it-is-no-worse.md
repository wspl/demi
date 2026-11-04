---
status: accepted
---

# Two devices talk directly only when it is no worse than the relay

## Problem

Alex forwards port 8800 from a work laptop to a Linux box at home, where a
web app runs. The laptop's network sends all its UDP through a VPN client's
tunnel, and so does the home box's. A page of the app takes minutes to load
on the laptop.

The two runners reach each other directly: their WebRTC peer connects, and
Settings › Port Forwarding says *Direct · 1 connection*. The path it found
goes through the tunnels and loses most of what it carries. Measured on such
a pair, downloading an 8 MB file through the forward:

| Path | Time | Speed |
| --- | --- | --- |
| The pair's peer | 63 s | 0.13 MB/s |
| The relay, through the backend | 4 s | 2.1 MB/s |

SCTP, which carries the peer's channels, halves its window at each loss, so
on a lossy path it settles near 1 Mbit/s whatever the link carries.

A pair's runners take the peer whenever it connects
(`crates/runner-direct/src/pairs.rs`): a forwarded connection opens on it,
and a pipe between the two devices moves to it (0114). A page does not: on a
device's Automatic route, it measures both paths and keeps the relay when the
direct one is worse (0030). Two devices have no such check, and no way to
leave a bad path but Relay Only, which the user must find and set.

## Decision

A pair of runners measures the direct path as a page does, before any
connection uses it, and takes it only when it is no worse than the relay.

The user sees what a page shows today. A forward whose pair's peer is worse
says, on its page in Settings › Port Forwarding:

```text
Status     Relay · 1 connection
           P2P is slower right now.
```

The pair tries again on the schedule a failed peer has: after 1, 2 and 5
minutes and then every 10, while it carries a connection.

### The measurement

When the pair's peer connects, the runner whose device id sorts first
measures it, as it already settles which of two crossing offers stands:

1. It sends 20 probes on each path, 100 ms apart, and counts one not
   answered within 2 seconds as lost.
   - On the peer, on a probe channel it opens, unordered and never
     retransmitted, which the other runner echoes as it echoes a page's.
   - Through the backend, as `pair_ping`, which the backend gives the other
     runner as the `direct_ping` a page sends, and whose `direct_pong` it
     gives back as `pair_pong`.
2. The direct path is worse when it lost more than one probe, or its median
   round trip is more than 20 ms above the relay's.
3. It tells the other runner on a channel of the peer, whose header is
   `measured`:

   ```rust
   pub enum PairHeader {
       // ...
       /// The verdict of the runner that measured the pair's peer: the
       /// other runner uses the peer only once it hears `slower: false`.
       Measured { slower: bool },
   }
   ```

4. On `slower: true`, it closes the peer; both runners give the pair the
   reason `slower` and keep its connections on the relay. On `false`, both
   take the peer as today.

Until the verdict, the peer is connecting on both sides: no connection opens
on it and no pipe moves to it. A measurement takes about 2 seconds, during
which the relay carries the pair as it does while the peer connects.

The figures and thresholds are those of a page's measurement, defined once
in `demi_runner_protocol::direct` and generated for the page, which reads
them from there instead of its own constants:

```rust
pub const MEASURE_PROBES: usize = 20;
pub const PROBE_EVERY: Duration = Duration::from_millis(100);
pub const PROBE_LOST: Duration = Duration::from_secs(2);
pub const WORSE_LOST_PROBES: usize = 1;
pub const WORSE_LATENCY: Duration = Duration::from_millis(20);
```

### The pair's route

A device's route says how pages and pairs reach it alike. The backend tells
a pair's runners one route for the pair, from the two devices':

| Either device | The pair's route |
| --- | --- |
| Is a Cloud, or is Relay Only | `relay`: no peer |
| Is Prefer P2P, and neither is Relay Only | `direct`: the peer is taken unmeasured |
| Otherwise | `automatic`: measured |

`DeviceRecord::may_talk_directly` becomes `DeviceRecord::pair_route`, and
the boolean the backend sends today becomes the route:

```rust
pub enum PairRoute {
    Relay,
    Automatic,
    Direct,
}
```

- `Forward::direct: bool` becomes `Forward::route: PairRoute`.
- A pipe's record names its sink's device to its source only when its two
  devices may talk directly; it names the route with it.
- `Pairs::path` takes the route in place of `allowed`.
- The backend's introduction of one runner's offer to the other,
  `Introduced::Runner`, names the route too: a forward's connecting end and
  a pipe's sink never ask for a path, and either may be the runner that
  measures.

`PairReason` gains `Slower`, which the forward's state carries as it carries
the others, and Settings › Port Forwarding words with the sentence a device
already has for a page, "P2P is slower right now."

Every part of a deployment runs one release, and a paired device's runner
updates to its backend's on its next connection, so no runner meets another
runner's protocol for longer than an update.

### What it does not catch

A path that answers 20 probes but loses data once a transfer fills it, such
as UDP a network limits by rate, passes the measurement, and its connections
stay slow until the peer fails. The measurement sends a few kilobytes so that
it can run at every peer's connection; a test of throughput would send
megabytes through the user's server each time, which 0030 declined for pages
too.

## Spike

Each point ran on two of the user's devices, a work laptop behind a VPN
client and a Linux box at home, and on a Mac at home.

- **Which path is slow.** The laptop and the home box each reach the backend
  at 0.7 to 2.4 MB/s. Through the forward, the pair's peer carried 0.13 MB/s
  and the relay 1.5 to 2.1 MB/s, the latter even through a second relay hop
  by way of the Cloud. The Mac and the home box, on one network, carried
  16.6 MB/s on their peer.
- **Whether probes see it.** Both the laptop and the Mac send their UDP
  through the same VPN exit, which STUN shows as one public address for both.
  A hole punched between them answered its first packets; then 0 of 20
  probes, sent as above, came back, and 8 of 4,062 datagrams sent at 1, 4 and
  8 Mbit/s arrived. The measurement finds such a path worse at once.

## Prior art

- Tailscale's `magicsock` uses a direct UDP path whenever one answers, and
  DERP, its relay, only while none does; among direct paths it prefers the
  lower latency
  ([`betterAddr`](https://github.com/tailscale/tailscale/blob/main/wgengine/magicsock/endpoint.go)).
- iroh does the same: a direct path once a ping on it is answered, the relay
  until then ([iroh paths](https://www.iroh.computer/docs/concepts/holepunching)).
- libp2p's DCUtR upgrades a relayed connection to a direct one once a hole is
  punched, without comparing them
  ([spec](https://github.com/libp2p/specs/blob/master/relay/DCUtR.md)).
- WebRTC's ICE ranks a relayed candidate below the others and never compares
  their quality (RFC 8445).

Each takes a path that answers. Demi departs, as its page already does,
because its relay is not a fallback of last resort: it runs over the
backend's own TCP link, which a VPN tunnel carries well while it loses UDP.

## Alternatives

- **Leave it to Relay Only.** The user would have to know that the path is
  the cause, and which device to set. A page measures for them already.
- **Each runner measures, and uses the peer for its own connections only.**
  Two measurements can disagree near a threshold, and a runner that found the
  peer worse could not close it while the other used it. One verdict keeps
  one state for the pair.
- **Measure throughput.** It catches the rate-limited path the probes miss,
  at megabytes through the user's server for every peer.
- **Measure again while a connection runs direct, and move back when it
  slows.** A move to the relay loses no byte (0114), but deciding "slow"
  needs a throughput the relay was not measured at. Left until a case needs
  it.

## Open questions

None.
