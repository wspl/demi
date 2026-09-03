// The kernel command line as the guest's configuration (`managed-hosts.md`
// § Joining): the backend passes what a guest needs at boot — where the
// backend is, the token that admits the guest, the network — as `demi.*`
// parameters, fresh at every spawn, so nothing of it touches persistent disk.

export interface GuestBootConfig {
  backendUrl: string
  deviceToken: string
  network: GuestNetwork | null
}

export interface GuestNetwork {
  /** The guest's address with its prefix length, `172.16.5.2/30`. */
  address: string
  gateway: string
  /** Nameservers, in order; empty when the command line named none. */
  dns: string[]
}

/** `/proc/cmdline` as key/value pairs; a bare word maps to the empty string. Quoted values (`key="a b"`) are one value. */
export function parseKernelCmdline(text: string): Map<string, string> {
  const params = new Map<string, string>()
  for (const word of text.trim().match(/(?:[^\s"]+|"[^"]*")+/g) ?? []) {
    const eq = word.indexOf('=')
    if (eq < 0) {
      params.set(word, '')
      continue
    }
    let value = word.slice(eq + 1)
    if (value.startsWith('"') && value.endsWith('"')) value = value.slice(1, -1)
    params.set(word.slice(0, eq), value)
  }
  return params
}

/** The guest's configuration out of the command line; missing backend or token is a fatal boot error. */
export function guestBootConfig(cmdline: string): GuestBootConfig {
  const params = parseKernelCmdline(cmdline)
  const backendUrl = params.get('demi.backend')
  const deviceToken = params.get('demi.token')
  if (!backendUrl) throw new Error('kernel command line names no demi.backend')
  if (!deviceToken) throw new Error('kernel command line names no demi.token')
  const address = params.get('demi.ip')
  const gateway = params.get('demi.gw')
  const network: GuestNetwork | null =
    address && gateway ? { address, gateway, dns: (params.get('demi.dns') ?? '').split(',').filter((entry) => entry.length > 0) } : null
  return { backendUrl, deviceToken, network }
}
