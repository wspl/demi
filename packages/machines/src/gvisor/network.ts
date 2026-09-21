import { lookup } from 'node:dns/promises'
import { writeFile } from 'node:fs/promises'
import ipaddr from 'ipaddr.js'
import { z } from 'zod'
import type { GVisorConfig } from './config'
import type { Slot } from './slots'
import { requireTool, runTool } from './image-tools'

const routesSchema = z.array(z.object({ dst: z.string().optional(), dev: z.string().optional() }))
const TABLE = 'demi_cloud'
const DENIED = ['0.0.0.0/8', '10.0.0.0/8', '100.64.0.0/10', '127.0.0.0/8', '169.254.0.0/16', '172.16.0.0/12', '192.0.0.0/24', '192.0.2.0/24', '192.168.0.0/16', '198.18.0.0/15', '198.51.100.0/24', '203.0.113.0/24', '224.0.0.0/4', '240.0.0.0/4']

/** Own host-side connectivity and egress rules for Cloud, independent of workloads. */
export class CloudNetwork {
  private backend: string[] = []
  private readonly url: URL
  constructor(private readonly config: GVisorConfig) {
    this.url = new URL(config.backendUrl)
  }

  async prepare(): Promise<void> {
    const routes = routesSchema.parse(JSON.parse(await requireTool('ip', ['-json', 'route', 'show'])))
    const own = ipaddr.IPv4.parseCIDR(this.config.subnet)
    for (const route of routes) {
      if (!route.dst || route.dst === 'default' || route.dev?.startsWith('demih')) continue
      const destination = route.dst.includes('/') ? route.dst : `${route.dst}/32`
      if (!ipaddr.IPv4.isValidCIDR(destination)) continue
      const other = ipaddr.IPv4.parseCIDR(destination)
      if (own[0].match(other) || other[0].match(own)) {
        throw new Error(`DEMI_MANAGED_SUBNET overlaps host route ${route.dst}`)
      }
    }
    this.backend = (await lookup(this.url.hostname, { family: 4, all: true })).map(entry => z.ipv4().parse(entry.address))
    if (!this.backend.length) throw new Error('Backend has no reachable IPv4 address')
    if (this.backend.some(address => ['loopback', 'unspecified'].includes(ipaddr.IPv4.parse(address).range()))) {
      throw new Error('Backend URL must be reachable from Cloud, not host loopback')
    }
    await writeFile('/proc/sys/net/ipv4/ip_forward', '1')
    const existing = await runTool('nft', ['list', 'table', 'inet', TABLE])
    const replace = existing.code === 0 ? `delete table inet ${TABLE}\n` : ''
    await requireTool('nft', ['-f', '-'], `${replace}table inet ${TABLE} {
      chain input { type filter hook input priority -10; policy accept; iifname "demih*" jump ingress; }
      chain forward { type filter hook forward priority -10; policy accept; iifname "demih*" jump ingress; oifname "demih*" ct state established,related accept; oifname "demih*" drop; }
      chain ingress { drop; }
      chain nat { type nat hook postrouting priority srcnat; policy accept; ip saddr ${this.config.subnet} oifname != "demih*" masquerade; }
    }`)
  }

  async create(slot: Slot): Promise<void> {
    await requireTool('ip', ['netns', 'add', slot.namespace])
    await requireTool('ip', ['link', 'add', slot.hostInterface, 'type', 'veth', 'peer', 'name', slot.peerInterface])
    await requireTool('ip', ['link', 'set', slot.peerInterface, 'netns', slot.namespace])
    await requireTool('ip', ['address', 'add', `${slot.gateway}/30`, 'dev', slot.hostInterface])
    await requireTool('ip', ['link', 'set', slot.hostInterface, 'up'])
    await writeFile(`/proc/sys/net/ipv6/conf/${slot.hostInterface}/disable_ipv6`, '1')
    await requireTool('ip', ['netns', 'exec', slot.namespace, 'sysctl', '-qw', 'net.ipv6.conf.all.disable_ipv6=1'])
    for (const args of [
      ['link', 'set', 'lo', 'up'], ['address', 'add', `${slot.address}/30`, 'dev', slot.peerInterface],
      ['link', 'set', slot.peerInterface, 'up'], ['route', 'add', 'default', 'via', slot.gateway],
    ]) await requireTool('ip', ['-n', slot.namespace, ...args])
    const port = this.url.port || (this.url.protocol === 'https:' ? '443' : '80')
    await requireTool('nft', ['-f', '-'], `add chain inet ${TABLE} slot${slot.index}
      add rule inet ${TABLE} slot${slot.index} meta nfproto != ipv4 drop
      add rule inet ${TABLE} slot${slot.index} ip saddr != ${slot.address} drop
      add rule inet ${TABLE} slot${slot.index} ip daddr { ${this.backend.join(', ')} } tcp dport ${port} accept
      add rule inet ${TABLE} slot${slot.index} ip daddr { ${this.config.dns.join(', ')} } meta l4proto { tcp, udp } th dport 53 accept
      add rule inet ${TABLE} slot${slot.index} fib daddr type local drop
      add rule inet ${TABLE} slot${slot.index} ip daddr { ${DENIED.join(', ')} } drop
      add rule inet ${TABLE} slot${slot.index} accept
      insert rule inet ${TABLE} ingress iifname "${slot.hostInterface}" jump slot${slot.index} comment "demi-slot-${slot.index}"`)
  }

  async remove(slot: Slot): Promise<void> {
    const listed = await runTool('nft', ['-j', 'list', 'table', 'inet', TABLE])
    if (listed.code === 0) {
      const table = z.object({ nftables: z.array(z.object({
        rule: z.object({ chain: z.string(), handle: z.number().int(), comment: z.string().optional() }).optional(),
        chain: z.object({ name: z.string() }).optional(),
      })) }).parse(JSON.parse(listed.stdout))
      const commands = table.nftables.flatMap(item => item.rule?.comment === `demi-slot-${slot.index}`
        ? [`delete rule inet ${TABLE} ${item.rule.chain} handle ${item.rule.handle}`] : [])
      if (table.nftables.some(item => item.chain?.name === `slot${slot.index}`)) {
        commands.push(`flush chain inet ${TABLE} slot${slot.index}`, `delete chain inet ${TABLE} slot${slot.index}`)
      }
      if (commands.length) await requireTool('nft', ['-f', '-'], commands.join('\n'))
    } else if (!listed.stderr.includes('No such file')) {
      throw new Error(`Cannot inspect Cloud network policy: ${listed.stderr}`)
    }
    // Namespace/interface queries distinguish absent resources from failed deletion.
    const namespaces = await requireTool('ip', ['netns', 'list'])
    if (namespaces.split('\n').some(line => line.split(' ')[0] === slot.namespace)) {
      await requireTool('ip', ['netns', 'delete', slot.namespace])
    }
    const links = z.array(z.object({ ifname: z.string() }))
    const present = async () => links.parse(JSON.parse(await requireTool('ip', ['-json', 'link', 'show'])))
      .some(link => link.ifname === slot.hostInterface)
    if (!await present()) return
    const deleted = await runTool('ip', ['link', 'delete', slot.hostInterface])
    // Namespace destruction removes its veth pair asynchronously.
    if (deleted.code !== 0 && await present()) throw new Error(`Cannot remove Cloud interface: ${deleted.stderr}`)
  }
}
