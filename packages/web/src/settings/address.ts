import { computed } from 'vue'
import { z } from 'zod'
import { useRoute, useRouter, type RouteLocationNormalizedLoaded, type Router } from 'vue-router'

/**
 * Settings' own addresses (`web-application.md` § Authentication):
 * `/settings/<section>` over the page settings opened from. Each section
 * the user opens is an entry of the history, so Back returns to the section
 * before, and from the first one closes settings; a reload keeps them open,
 * since the history keeps what each entry says it opened over. A link opens
 * its section over the chat. `/settings` alone is settings with no section
 * chosen: the list of sections on a phone, the first section elsewhere.
 */
export const SETTINGS_ROUTE = 'settings'

/** Where a settings entry was opened over, kept in the history with the entry. */
const settingsEntrySchema = z.object({
  /** The address of the page settings opened over. */
  settingsOver: z.string().startsWith('/'),
  /** How many settings entries the history holds above that page, this one included. */
  settingsDepth: z.number().int().positive(),
  /** The entry before this one is the list of sections, which a phone's back button returns to. */
  settingsFromList: z.boolean(),
})
type SettingsEntry = z.infer<typeof settingsEntrySchema>

/** The page settings open over when nothing says another: the chat. */
const DEFAULT_PAGE = '/chat'

/** What the history says of the current entry; null for an entry no settings navigation made, such as a link. */
function currentEntry(router: Router): SettingsEntry | null {
  const parsed = settingsEntrySchema.safeParse(router.options.history.state)
  return parsed.success ? parsed.data : null
}

/** The section the route opens: undefined while settings are closed, null for none chosen. */
export function routeSection(route: RouteLocationNormalizedLoaded): string | null | undefined {
  if (route.name !== SETTINGS_ROUTE) {
    return undefined
  }
  const section = route.params.section
  return typeof section === 'string' && section !== '' ? section : null
}

/** The address of the page under settings, which the window shows behind the dialog. */
export function pageUnderSettings(router: Router): string {
  return currentEntry(router)?.settingsOver ?? DEFAULT_PAGE
}

function location(section: string | null) {
  return section === null
    ? { name: SETTINGS_ROUTE }
    : { name: SETTINGS_ROUTE, params: { section } }
}

/**
 * Opens settings on `section`, or on none: over the current page when they
 * are closed, as the next entry of the history when they are open.
 */
export async function openSettings(router: Router, section: string | null = null): Promise<void> {
  const route = router.currentRoute.value
  if (routeSection(route) === undefined) {
    await router.push({
      ...location(section),
      state: { settingsOver: route.fullPath, settingsDepth: 1, settingsFromList: false } satisfies SettingsEntry,
    })
    return
  }
  await showSection(router, section)
}

/**
 * Shows `section` in open settings, as a new entry. Going back to the list,
 * as a phone's back button does, returns to the list's entry when it is the
 * one before, and otherwise takes this entry's place.
 */
export async function showSection(router: Router, section: string | null): Promise<void> {
  const route = router.currentRoute.value
  if (routeSection(route) === section) {
    return
  }
  const entry = currentEntry(router) ?? { settingsOver: DEFAULT_PAGE, settingsDepth: 1, settingsFromList: false }
  if (section === null && entry.settingsFromList) {
    router.back()
    return
  }
  if (section === null) {
    await router.replace({ ...location(null), state: entry })
    return
  }
  await router.push({
    ...location(section),
    state: {
      settingsOver: entry.settingsOver,
      settingsDepth: entry.settingsDepth + 1,
      settingsFromList: routeSection(route) === null,
    } satisfies SettingsEntry,
  })
}

/**
 * Closes settings: back past every settings entry to the page they opened
 * over, so Back afterwards leaves that page as it would have; settings a
 * link opened take their own entry's place with the chat.
 */
export async function closeSettings(router: Router): Promise<void> {
  const entry = currentEntry(router)
  if (entry) {
    router.go(-entry.settingsDepth)
    return
  }
  await router.replace(DEFAULT_PAGE)
}

/** The settings address for a component: the open section, and the ways to change it. */
export function useSettingsAddress() {
  const router = useRouter()
  const route = useRoute()
  return {
    /** The open section: undefined while settings are closed, null for none chosen. */
    section: computed(() => routeSection(route)),
    open: (section?: string) => openSettings(router, section ?? null),
    show: (section: string | null) => showSection(router, section),
    close: () => closeSettings(router),
  }
}
