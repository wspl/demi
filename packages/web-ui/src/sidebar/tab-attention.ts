import { computed, effectScope, ref, shallowRef, watch, watchEffect } from 'vue'
import { followAppearance, pageTopColor } from '../theme/themeColor'
import { conversationDotTone } from './conversation-dot'
import { t } from './locales/text'
import type { SidebarConversation } from './types'

/**
 * The browser tab's news (`conversations.md` § Conversations and projects):
 * the page's title counts the conversations whose sidebar dot is red, yellow
 * or green, the tab's icon wears a dot of the most urgent of those colors,
 * and an installed app's icon carries their number. All of it follows the
 * sidebar's dots, read from the same rows by the same rule.
 */

/** The product's name, as the tab says it. */
const PRODUCT_NAME = 'demi'

/** The dots that say something is new, the most urgent first. */
const NEWS = ['danger', 'attention', 'success'] as const
type News = (typeof NEWS)[number]

/** How many conversations wear each dot that says something is new. */
type TabNews = Readonly<Record<News, number>>

/** The page's title without news: a conversation's title and the product's name, or the name alone. */
export function pageTitle(title: string | null): string {
  return title === null ? PRODUCT_NAME : `${title} · ${PRODUCT_NAME}`
}

/**
 * The page's title: while anything is new, one count first, of the one kind
 * there is, or of every kind as to do once there are several, since one
 * short count reads at a glance where a list does not.
 */
function tabTitle(news: TabNews, title: string | null): string {
  const present = NEWS.filter((kind) => news[kind] > 0)
  const total = present.reduce((sum, kind) => sum + news[kind], 0)
  if (total === 0)
    return pageTitle(title)
  const kind = present.length === 1 ? present[0] : 'attention'
  return `(${t(`tab.${kind}`, { count: total })}) ${pageTitle(title)}`
}

/** The plain icon with a dot of `fill` in its lower right corner, ringed with `ring`, as a data URL. */
function dottedIcon(svg: string, fill: string, ring: string): string {
  const icon = new DOMParser().parseFromString(svg, 'image/svg+xml')
  const root = icon.documentElement
  const [x = 0, y = 0, width = 0, height = 0] = (root.getAttribute('viewBox') ?? '').split(/[\s,]+/).map(Number)
  const size = Math.min(width, height)
  // In the very corner, touching the icon's bottom and right edges, and
  // small, so it covers little of the mark: on a 16-pixel tab icon a dot of
  // 4 pixels in a ring of under 1.
  const outer = size * 0.165
  const cx = x + width - outer
  const cy = y + height - outer
  for (const [radius, color] of [[outer, ring], [size * 0.125, fill]] as const) {
    const circle = icon.createElementNS('http://www.w3.org/2000/svg', 'circle')
    circle.setAttribute('cx', String(cx))
    circle.setAttribute('cy', String(cy))
    circle.setAttribute('r', String(radius))
    circle.setAttribute('fill', color)
    root.append(circle)
  }
  return `data:image/svg+xml,${encodeURIComponent(new XMLSerializer().serializeToString(icon))}`
}

/** The theme's dot colors, the sidebar's tokens, and the tab strip's background, which the page's top color is. */
function readColors(): Readonly<Record<News | 'ring', string>> {
  const style = getComputedStyle(document.documentElement)
  const token = (name: string) => style.getPropertyValue(name).trim()
  return {
    danger: token('--on-danger'),
    attention: token('--on-attention'),
    success: token('--on-success'),
    ring: pageTopColor(),
  }
}

/** What the tab tells of: the rows the sidebar lists, the open one, and its title. */
export interface TabAttentionSource {
  /** The sidebar's rows; none on a page that shows no sidebar, such as a sign-in page. */
  conversations: () => readonly Pick<SidebarConversation, 'id' | 'status' | 'needsYou' | 'unread'>[]
  /** The conversation open in the page, which a focused page reads as it happens. */
  openId: () => string | null
  /** The open conversation's title as it shows, or null where none is open. */
  openTitle: () => string | null
}

/** The channel on which a page that comes forward tells the user's other pages, so the badge follows it. */
const FORWARD_CHANNEL = 'demi-tab-forward'

/**
 * Keeps the page's title, its tab icon and an installed app's badge on the
 * sidebar's news as it changes; returns the release, which leaves the plain
 * icon and no badge.
 *
 * Coming back clears the reminder: when the page comes forward, visible and
 * focused, it counts nothing, and from then on only the conversations whose
 * dot lights after that moment, while the sidebar's dots stay until each
 * conversation is opened. The badge follows the page that came forward last,
 * which tells the user's other pages so on a broadcast channel.
 */
export function followTabAttention(source: TabAttentionSource): () => void {
  const scope = effectScope(true)
  const inFront = () => document.visibilityState === 'visible' && document.hasFocus()
  const front = ref(inFront())
  /** The conversations whose dot lit since the page was last in front. */
  const lit = shallowRef<ReadonlySet<string>>(new Set())
  /** Whether the page has been in front since it opened: until then every lit dot is news. */
  let beenInFront = front.value
  /** The page writes the badge while it is the one that came forward last. */
  const badgeOwner = ref(front.value)
  const forward = new BroadcastChannel(FORWARD_CHANNEL)
  // Any message is another page coming forward; it carries nothing to read.
  forward.onmessage = () => {
    badgeOwner.value = false
  }
  const followFront = () => {
    const now = inFront()
    if (now && !front.value) {
      beenInFront = true
      lit.value = new Set()
      badgeOwner.value = true
      forward.postMessage(null)
    }
    front.value = now
  }
  window.addEventListener('focus', followFront)
  window.addEventListener('blur', followFront)
  document.addEventListener('visibilitychange', followFront)
  const colors = shallowRef(readColors())
  const stopAppearance = followAppearance(() => {
    colors.value = readColors()
  })
  const plainHref = document.head.querySelector('link[rel="icon"]')?.getAttribute('href') ?? null
  let plainIcon: Promise<string> | null = null
  // The latest icon asked for: one whose SVG arrives after a later change is dropped.
  let iconRequest = 0

  /** Puts `href` on a new icon link in place of the old one, which every browser redraws. */
  const showIcon = (href: string): void => {
    const link = document.head.querySelector('link[rel="icon"]')
    if (!link || link.getAttribute('href') === href)
      return
    const next = document.createElement('link')
    for (const attribute of link.attributes)
      next.setAttribute(attribute.name, attribute.value)
    next.setAttribute('href', href)
    link.replaceWith(next)
  }
  const showDot = (fill: string | null, ring: string): void => {
    const request = ++iconRequest
    if (plainHref === null)
      return
    if (fill === null) {
      showIcon(plainHref)
      return
    }
    // The icon the page's HTML names, read once it first wears a dot.
    plainIcon ??= fetch(plainHref).then((response) =>
      response.ok ? response.text() : Promise.reject(new Error(`HTTP ${response.status}`)))
    plainIcon.then(
      (svg) => {
        if (request === iconRequest)
          showIcon(dottedIcon(svg, fill, ring))
      },
      (error: unknown) => {
        // The tab keeps its plain icon, and the title still says what is new;
        // the next change reads the icon again.
        plainIcon = null
        console.warn('The tab icon was not read', error)
      },
    )
  }
  const showBadge = (count: number): void => {
    if (!('setAppBadge' in navigator))
      return
    // The badge is a courtesy: a browser that refuses it, as some do for a page
    // that is not installed, shows none, and the title and the icon still say it.
    void (count > 0 ? navigator.setAppBadge(count) : navigator.clearAppBadge()).catch(() => {})
  }

  scope.run(() => {
    /** Each listed conversation's news, by its sidebar dot: red, yellow or green, or none. */
    const tones = computed(() => new Map(source.conversations().map((conversation) => {
      const tone = conversationDotTone(conversation)
      return [conversation.id, tone === 'danger' || tone === 'attention' || tone === 'success' ? tone : null]
    })))
    // A dot lights when it takes a color of news it did not wear: green after the
    // running blue, red after green. A row the page sees for the first time, as
    // the list arrives, lit before the page came forward, so it is news only to a
    // page never in front; a new conversation arrives settled and lights later.
    watch(tones, (now, before) => {
      const next = new Set([...lit.value].filter((id) => now.get(id)))
      for (const [id, tone] of now) {
        const lights = before?.has(id) ? before.get(id) !== tone : !beenInFront
        if (tone && lights)
          next.add(id)
      }
      if (next.size !== lit.value.size || [...next].some((id) => !lit.value.has(id)))
        lit.value = next
    }, { immediate: true })
    const news = computed<TabNews>(() => {
      const read = front.value ? source.openId() : null
      const counts = { danger: 0, attention: 0, success: 0 }
      for (const id of lit.value) {
        const tone = tones.value.get(id)
        if (tone && id !== read)
          counts[tone] += 1
      }
      return counts
    })
    watchEffect(() => {
      document.title = tabTitle(news.value, source.openTitle())
    })
    const urgent = computed(() => NEWS.find((kind) => news.value[kind] > 0) ?? null)
    watch(
      [() => (urgent.value ? colors.value[urgent.value] : null), () => colors.value.ring],
      ([fill, ring]) => showDot(fill, ring),
      { immediate: true },
    )
    watch(
      [badgeOwner, () => news.value.danger + news.value.attention + news.value.success],
      ([owner, count]) => {
        if (owner)
          showBadge(count)
      },
      { immediate: true },
    )
  })

  return () => {
    scope.stop()
    stopAppearance()
    forward.close()
    window.removeEventListener('focus', followFront)
    window.removeEventListener('blur', followFront)
    document.removeEventListener('visibilitychange', followFront)
    showDot(null, '')
    if (badgeOwner.value)
      showBadge(0)
  }
}
