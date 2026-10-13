import type { DeviceReport } from '@demicodes/web-ui/auth/device-unlock'

/** The browsers of each system the passkey offer names the unlock of, as their pages report themselves. */
export const DEVICE_REPORTS: Record<string, { label: string; report: DeviceReport }> = {
  ios: {
    label: 'iPhone',
    report: {
      userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
      maxTouchPoints: 5,
    },
  },
  mac: {
    label: 'Mac',
    report: {
      userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15',
      maxTouchPoints: 0,
    },
  },
  windows: {
    label: 'Windows',
    report: {
      userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0',
      maxTouchPoints: 0,
    },
  },
  android: {
    label: 'Android',
    report: {
      userAgent: 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36',
      maxTouchPoints: 5,
    },
  },
  other: {
    label: 'Linux',
    report: {
      userAgent: 'Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0',
      maxTouchPoints: 0,
    },
  },
}
