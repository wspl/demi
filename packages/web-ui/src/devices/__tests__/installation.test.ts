import { expect, test } from 'bun:test'
import { deviceInstallationAt, deviceInstallCommand, type DeviceSystem } from '../installation'

// The command the page shows fetches the installer from the backend's public
// URL, at its origin, over that URL's own scheme: a development backend
// serves plain HTTP, which a command that insists on HTTPS refuses.
const cases: { publicUrl: string; system: DeviceSystem; command: string }[] = [
  {
    publicUrl: 'https://demi.example.com/',
    system: 'linux',
    command: "curl --proto '=https' --tlsv1.2 -sSf 'https://demi.example.com/install.sh' | sh",
  },
  {
    publicUrl: 'http://192.168.5.2:3271/',
    system: 'macos',
    command: "curl --proto '=http' -sSf 'http://192.168.5.2:3271/install.sh' | sh",
  },
  {
    publicUrl: 'https://demi.example.com/api/runner',
    system: 'linux',
    command: "curl --proto '=https' --tlsv1.2 -sSf 'https://demi.example.com/install.sh' | sh",
  },
  {
    publicUrl: 'http://192.168.5.2:3271/',
    system: 'windows',
    command: "irm 'http://192.168.5.2:3271/install.ps1' | iex",
  },
]

test('the install command names the backend public URL and allows its scheme', () => {
  for (const { publicUrl, system, command } of cases) {
    expect(deviceInstallCommand(deviceInstallationAt(publicUrl), system)).toBe(command)
  }
})
