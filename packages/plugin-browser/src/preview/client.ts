/**
 * The user's browser as its own requests describe it (`preview.md`
 * § Upstream requests): the engine requests upstream with its User-Agent,
 * client hints and languages, so a site sees the browser the preview shows
 * it in.
 */
import type { PreviewClient } from '../generated/plugin'

/** The User-Agent Client Hints of Chromium, which the DOM's types do not declare yet. */
interface UserAgentData {
  brands: { brand: string; version: string }[]
  mobile: boolean
  platform: string
}

function userAgentData(): UserAgentData | undefined {
  return (navigator as Navigator & { userAgentData?: UserAgentData }).userAgentData
}

/** `Accept-Language` as Chrome builds it from the browser's languages. */
export function acceptLanguage(languages: readonly string[]): string {
  return languages
    .map((language, index) => (index === 0 ? language : `${language};q=${Math.max(0.1, 1 - index / 10).toFixed(1)}`))
    .join(',')
}

/** This browser, as a preview's requests describe it. */
export function previewClient(): PreviewClient {
  const data = userAgentData()
  const languages = navigator.languages.length > 0 ? navigator.languages : [navigator.language]
  return {
    userAgent: navigator.userAgent,
    brands: data?.brands.map(({ brand, version }) => `"${brand}";v="${version}"`).join(', ') ?? '',
    mobile: data?.mobile ?? false,
    platform: data?.platform ?? '',
    acceptLanguage: acceptLanguage(languages),
  }
}

/**
 * Why this page cannot show a preview, or null when it can
 * (`preview.md` § Where the preview cannot run): the forwarder is a service
 * worker, which only a secure context registers, and the preview is built
 * and measured in Chrome.
 */
export function previewUnsupported(): string | null {
  if (!globalThis.isSecureContext) {
    return 'Previews need Demi opened over HTTPS or on localhost.'
  }
  // Only Chromium's browsers describe themselves with client hints; a headless one names no brands.
  if (!userAgentData() || !('serviceWorker' in navigator)) {
    return 'Previews need Google Chrome or another Chromium browser.'
  }
  return null
}
