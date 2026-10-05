import { expect, test } from 'bun:test'
import { clientPlatform } from '@demicodes/utils'
import {
  deviceInstallationAt,
  deviceInstallCommand,
  deviceSystemOf,
  type DeviceSystem,
} from '../installation'

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

// A person usually adds the computer they are using, so the dialog starts on
// the browser's own system; one it cannot tell starts on Linux.
test('the dialog starts on the system of the browser it opens in', () => {
  const browsers: [string, string, DeviceSystem][] = [
    ['MacIntel', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', 'macos'],
    ['Win32', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'windows'],
    ['Linux x86_64', 'Mozilla/5.0 (X11; Linux x86_64)', 'linux'],
    ['', 'Mozilla/5.0 (SomethingElse)', 'linux'],
  ]
  for (const [platform, userAgent, system] of browsers) {
    expect(deviceSystemOf(clientPlatform({ platform, userAgent }))).toBe(system)
  }
})
