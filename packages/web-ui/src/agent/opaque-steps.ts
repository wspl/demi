import type { SearchAction } from '@demicodes/protocol'
import { t } from './locales/text'

/**
 * What a vendor's search did, as its row names it: the same while it runs
 * and once it ended.
 */
export function searchTitle(action: SearchAction): string {
  return t(`transcript.opaque.search.${action.type}`)
}

/** The words a vendor's search went by: its queries, or the page it read. */
export function searchSubject(action: SearchAction): string {
  switch (action.type) {
    case 'web':
      return action.queries.join(' · ')
    case 'open_page':
      return sourcePage(action.url)
    case 'find_in_page':
      return `${action.pattern} · ${sourcePage(action.url)}`
    case 'x':
    case 'x_people':
      return action.query
    case 'x_post':
    case 'image':
    case 'x_video':
      return sourcePage(action.url)
  }
}

/**
 * The kind of work a search counts as in its run's row: the web's, a page
 * read, X's, or an image or a video looked at.
 */
export function searchKind(action: SearchAction): 'web' | 'page' | 'x' | 'media' {
  switch (action.type) {
    case 'web':
      return 'web'
    case 'open_page':
    case 'find_in_page':
      return 'page'
    case 'x':
    case 'x_people':
    case 'x_post':
      return 'x'
    case 'image':
    case 'x_video':
      return 'media'
  }
}

/** A source's site, its host without `www.`. */
export function sourceHost(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, '')
  } catch {
    return url
  }
}

/** A page as a row names it: its host and path, without the scheme. */
export function sourcePage(url: string): string {
  try {
    const parsed = new URL(url)
    return `${parsed.host.replace(/^www\./, '')}${parsed.pathname === '/' ? '' : parsed.pathname}`
  } catch {
    return url
  }
}
