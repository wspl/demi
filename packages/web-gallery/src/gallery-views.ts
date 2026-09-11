import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'

export type GalleryViewOption = {
  value: string
  label: string
}

export const GALLERY_VIEWS: Record<string, readonly GalleryViewOption[]> = {
  '/errors': [
    { value: 'primitives', label: 'Primitives' },
    { value: 'conversation', label: 'Conversation' },
    { value: 'messages', label: 'Messages & uploads' },
    { value: 'forms', label: 'Forms & settings' },
    { value: 'files', label: 'Files' },
  ],
  '/overview': [
    {
      value: 'overview',
      label: 'Overview',
    },
  ],
  '/signin': [
    {
      value: 'signin',
      label: 'Sign in',
    },
  ],
  '/surfaces': [
    {
      value: 'surfaces',
      label: 'Surfaces',
    },
  ],
  '/primitives': [
    {
      value: 'buttons',
      label: 'Buttons',
    },
    {
      value: 'fields',
      label: 'Fields',
    },
    {
      value: 'marks',
      label: 'Marks',
    },
  ],
  '/motion': [
    {
      value: 'motion',
      label: 'Motion',
    },
  ],
  '/overlays': [
    {
      value: 'menus',
      label: 'Menus',
    },
    {
      value: 'dialogs',
      label: 'Dialogs',
    },
  ],
  '/files': [
    {
      value: 'browser',
      label: 'Browser',
    },
    {
      value: 'dialogs',
      label: 'Dialogs',
    },
  ],
  '/session': [
    {
      value: 'tabs',
      label: 'Tab bar',
    },
    {
      value: 'composer',
      label: 'Composer',
    },
    {
      value: 'blocks',
      label: 'Blocks',
    },
    {
      value: 'turns',
      label: 'Turns',
    },
    {
      value: 'states',
      label: 'States',
    },
    {
      value: 'session',
      label: 'Session',
    },
    {
      value: 'windows',
      label: 'Windows',
    },
  ],
  '/sidebar': [
    {
      value: 'sidebar',
      label: 'Sidebar',
    },
  ],
  '/settings': [
    {
      value: 'settings',
      label: 'Settings',
    },
    {
      value: 'account',
      label: 'Account',
    },
    {
      value: 'device',
      label: 'Device',
    },
    {
      value: 'signin',
      label: 'Sign in',
    },
  ],
  '/markdown': [
    {
      value: 'markdown',
      label: 'Markdown',
    },
  ],
  '/code': [
    {
      value: 'code',
      label: 'Code',
    },
  ],
  '/roadmap': [
    {
      value: 'roadmap',
      label: 'Roadmap',
    },
  ],
}

export function viewsFor(path: string): readonly GalleryViewOption[] {
  return (
    GALLERY_VIEWS[path] ?? [
      {
        value: 'page',
        label: 'Page',
      },
    ]
  )
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
