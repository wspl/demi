import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'
import { t } from './locales/text'

export type GalleryViewOption = {
  value: string
  label: TitleText
}

/** The views of each page, labelled in the page's language each time they are read. */
function galleryViews(): Record<string, readonly GalleryViewOption[]> {
  return {
    '/errors': [
      { value: 'primitives', label: t('views.errors.primitives') },
      { value: 'conversation', label: t('views.errors.conversation') },
      { value: 'messages', label: t('views.errors.messages') },
      { value: 'forms', label: t('views.errors.forms') },
      { value: 'files', label: t('views.errors.files') },
    ],
    '/overview': [{ value: 'overview', label: t('nav.overview') }],
    '/signin': [
      { value: 'signin', label: t('views.signin.signin') },
      { value: 'setup', label: t('views.signin.setup') },
      { value: 'handoff', label: t('views.signin.handoff') },
      { value: 'starting', label: t('views.signin.starting') },
    ],
    '/surfaces': [{ value: 'surfaces', label: t('nav.surfaces') }],
    '/primitives': [
      { value: 'buttons', label: t('views.primitives.buttons') },
      { value: 'fields', label: t('views.primitives.fields') },
      { value: 'marks', label: t('views.primitives.marks') },
    ],
    '/writing': [{ value: 'writing', label: t('nav.writing') }],
    '/motion': [{ value: 'motion', label: t('nav.motion') }],
    '/overlays': [
      { value: 'menus', label: t('views.overlays.menus') },
      { value: 'dialogs', label: t('views.overlays.dialogs') },
    ],
    '/files': [
      { value: 'browser', label: t('views.files.browser') },
      { value: 'tree', label: t('views.files.tree') },
      { value: 'dialogs', label: t('views.files.dialogs') },
    ],
    '/session': [
      { value: 'tabs', label: t('views.session.tabs') },
      { value: 'composer', label: t('views.session.composer') },
      { value: 'blocks', label: t('views.session.blocks') },
      { value: 'lengths', label: t('views.session.lengths') },
      { value: 'changes', label: t('views.session.changes') },
      { value: 'turns', label: t('views.session.turns') },
      { value: 'states', label: t('views.session.states') },
      { value: 'session', label: t('views.session.session') },
      { value: 'panel', label: t('views.session.panel') },
      { value: 'windows', label: t('views.session.windows') },
      { value: 'shared', label: t('views.session.shared') },
    ],
    '/sidebar': [{ value: 'sidebar', label: t('nav.sidebar') }],
    '/automations': [
      { value: 'app', label: t('views.automations.app') },
      { value: 'phone', label: t('views.automations.phone') },
    ],
    '/settings': [{ value: 'settings', label: t('nav.settings') }],
    '/dialogs': [
      { value: 'account', label: t('views.dialogs.account') },
      { value: 'providers', label: t('views.dialogs.providers') },
      { value: 'devices', label: t('views.dialogs.devices') },
      { value: 'workspace', label: t('views.dialogs.workspace') },
      { value: 'cloud', label: t('views.dialogs.cloud') },
      { value: 'skills', label: t('views.dialogs.skills') },
      { value: 'search', label: t('views.dialogs.search') },
    ],
    '/markdown': [{ value: 'markdown', label: t('nav.markdown') }],
    '/phone': [{ value: 'phone', label: t('nav.phone') }],
    '/roadmap': [{ value: 'roadmap', label: t('nav.roadmap') }],
  }
}

export function viewsFor(path: string): readonly GalleryViewOption[] {
  return galleryViews()[path] ?? [{ value: 'page', label: t('views.page') }]
}

export function defaultView(path: string): string {
  if (path === '/session') {
    return 'session'
  }
  return viewsFor(path)[0]!.value
}

export function useGalleryView() {
  const route = useRoute()
  const router = useRouter()
  const views = computed(() => viewsFor(route.path))
  const view = computed({
    get() {
      const raw = route.query.view
      const value = Array.isArray(raw) ? raw[0] : raw
      if (
        typeof value === 'string' &&
        views.value.some((option) => option.value === value)
      ) {
        return value
      }
      return defaultView(route.path)
    },
    set(next: string) {
      void router.replace({
        query: next === defaultView(route.path) ? {} : { view: next },
      })
    },
  })
  return {
    views,
    view,
  }
}
