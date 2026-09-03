// Firecracker's API over its unix socket (`firecracker_spec` v1.16): the
// configuration before `InstanceStart`, the pause and resume around a
// checkpoint copy, and the drive rescan after the home file grew.
import { errorMessage } from '@demicodes/utils'

export interface VmDescription {
  vcpus: number
  memMib: number
  kernelPath: string
  bootArgs: string
  rootfsPath: string
  homePath: string
  tap: string
  mac: string
}

export class FirecrackerApi {
  constructor(private readonly socketPath: string) {}

  /** Waits for the socket to accept connections: the process is up. */
  async ready(timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs
    for (;;) {
      try {
        await this.request('GET', '/')
        return
      } catch (error) {
        if (Date.now() > deadline) throw new Error(`firecracker API did not come up: ${errorMessage(error)}`)
        await new Promise((resolve) => setTimeout(resolve, 20))
      }
    }
  }

  async configure(vm: VmDescription): Promise<void> {
    await this.request('PUT', '/machine-config', { vcpu_count: vm.vcpus, mem_size_mib: vm.memMib })
    await this.request('PUT', '/boot-source', { kernel_image_path: vm.kernelPath, boot_args: vm.bootArgs })
    await this.request('PUT', '/drives/rootfs', { drive_id: 'rootfs', path_on_host: vm.rootfsPath, is_root_device: true, is_read_only: true })
    await this.request('PUT', '/drives/home', { drive_id: 'home', path_on_host: vm.homePath, is_root_device: false, is_read_only: false })
    await this.request('PUT', '/network-interfaces/eth0', { iface_id: 'eth0', guest_mac: vm.mac, host_dev_name: vm.tap })
  }

  async start(): Promise<void> {
    await this.request('PUT', '/actions', { action_type: 'InstanceStart' })
  }

  async pause(): Promise<void> {
    await this.request('PATCH', '/vm', { state: 'Paused' })
  }

  async resume(): Promise<void> {
    await this.request('PATCH', '/vm', { state: 'Resumed' })
  }

  /** The backing file changed size: Firecracker re-reads it and the guest sees a bigger block device. */
  async rescanHome(homePath: string): Promise<void> {
    await this.request('PATCH', '/drives/home', { drive_id: 'home', path_on_host: homePath })
  }

  private async request(method: string, path: string, body?: unknown): Promise<void> {
    const response = await fetch(`http://firecracker${path}`, {
      method,
      unix: this.socketPath,
      headers: { accept: 'application/json', ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    })
    if (!response.ok) throw new Error(`firecracker ${method} ${path}: HTTP ${response.status} ${await response.text()}`)
  }
}
