import { z } from 'zod'

/**
 * A page in the user's own browser, in a sandboxed frame
 * (`web-application.md` § Work panel). Only an expose opens one; the tab then
 * loads whatever address the user submits.
 */
export const pageTabDataSchema = z.object({
  url: z.url(),
  /** The exposed address the tab still shows, which names it; null once the user went elsewhere. */
  expose: z.string().nullable(),
})

export type PageTabData = z.infer<typeof pageTabDataSchema>

/** The data of a tab an expose opens: its URL, named by the address it exposes. */
export function exposePageTab(expose: { url: string; address: string }): PageTabData {
  return { url: expose.url, expose: expose.address }
}

/**
 * The tab after the user submits `draft`: an `http` or `https` URL loads, a
 * draft without a scheme is tried as `https`, and anything else changes
 * nothing.
 */
export function loadPageAddress(data: PageTabData, draft: string): PageTabData {
  const trimmed = draft.trim()
  const candidate = trimmed.includes('://') ? trimmed : `https://${trimmed}`
  if (!trimmed || !URL.canParse(candidate)) {
    return data
  }
  const url = new URL(candidate)
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return data
  }
  return { url: url.href, expose: null }
}

/** An expose names its tab by the address it exposes; any other page by its host. */
export function pageTabTitle(data: PageTabData): string {
  return data.expose ?? new URL(data.url).host
}
