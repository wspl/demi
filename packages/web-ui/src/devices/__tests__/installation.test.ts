import { expect, test } from 'bun:test'
import { clientPlatform } from '@demicodes/utils'
import { deviceSystemOf, type DeviceSystem } from '../installation'

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
