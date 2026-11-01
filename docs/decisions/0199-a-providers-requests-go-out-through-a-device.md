---
status: proposed
---

# A provider's requests go out through a device

Supersedes the exposed ports, and the address that names a device by its id,
of the provider endpoints that reach a device.

## Problem

The user runs Ollama on a workstation at `192.168.1.50`, on the same office
network as their paired MacBook. Demi's backend is on a server outside that
network, so it cannot reach `192.168.1.50`. An exposed port only reaches
`127.0.0.1:<port>` on the device, at the address `http://<device id>:<port>`,
so the user has to run a forwarder of their own on the MacBook, such as
`socat TCP-LISTEN:11434,fork TCP:192.168.1.50:11434`, keep it running, and
expose its port. The exposed ports themselves are a second list on each
device's page, and an endpoint whose host is a device's id reads as nothing
the user would type.

## Decision

An API-key provider entry has Connect Through, under its Endpoint: the
server, as every entry has had, or one of its owner's devices. Through a
device, every request the entry makes, its turns, its model list, its media
and their downloads, goes out from that device's runner, which reaches the
endpoint as any program on the device reaches an address: `127.0.0.1` is the
device itself, and `192.168.1.50` or `nas.local` is on its network.

```text
Endpoint          http://192.168.1.50:11434/v1
Connect Through   [alex-mbp v]   Server / alex-mbp / build-01 (Offline)
                  Requests go out from alex-mbp, which reaches the
                  endpoint on its own network.
```

- The choice is a column of the entry, beside its endpoint; a device the
  owner removes leaves it, and the entry's requests fail with "The device
  this provider connects through was removed" until another is chosen.
- A request through a device that is offline, or a Cloud that is stopped,
  fails at once with "alex-mbp is offline", which no request wakes.
- On a shared instance an administrator's shared entry goes out through the
  administrator's device for every user of the entry.
- `demi providers` sets it as it sets an entry's endpoint, under the same
  permission.
- The runner reaches what any program on its device reaches: no list of
  ports or hosts limits it. The device is the owner's, who runs any command
  on it already.

The exposed ports go: their list on a device's page, `demi host expose` and
its permission with its grants, the Expose Port that a failed test or turn
offered, and the address `http://<device id>:<port>`. An entry whose endpoint
names one of its owner's devices that way connects through that device, its
endpoint `http://127.0.0.1:<port>` (or `https://localhost:<port>`), with the
rest of its path.

## Spike

None.

## Prior art

- ngrok's agent forwards to any address it can reach, such as
  `ngrok http 192.168.1.2:80`, not only to `localhost`
  ([Expose a service](https://ngrok.com/docs/gateway/agent/expose-a-service.md)).
- A Cloudflare Tunnel's ingress rule names its service by URL, which the
  machine running `cloudflared` resolves and reaches
  ([Ingress rules](https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/configuration/ingress)).
- An HTTP proxy setting, as macOS's network settings and every HTTP client
  take one, sends a client's requests out from another machine; Connect
  Through is that for one provider entry, with the user's own device in the
  proxy's place.

## Alternatives

- Exposed ports that forward to an address on the device's network, each
  with a port of its own on the device's address, as SSH's
  `-L 11435:192.168.1.50:11434`: two lists to keep in step, the device's and
  the provider's, for what one choice on the provider says.
- A subnet route, as Tailscale's subnet router advertises a network: the
  route is the whole backend's rather than one entry's.

## Open questions

None.
