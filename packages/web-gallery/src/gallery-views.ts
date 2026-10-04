import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'

export type GalleryViewOption = {
  value: string
  label: TitleText
}

export const GALLERY_VIEWS: Record<string, readonly GalleryViewOption[]> = {
  '/errors': [
    { value: 'primitives', label: 'Primitives' },
    { value: 'conversation', label: 'Conversation' },
    { value: 'messages', label: 'Messages & Uploads' },
    { value: 'forms', label: 'Forms & Settings' },
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
      label: 'Sign In',
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
  '/writing': [
    {
      value: 'writing',
      label: 'Writing',
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
      value: 'tree',
      label: 'Tree',
    },
    {
      value: 'dialogs',
      label: 'Dialogs',
    },
  ],
  '/session': [
    {
      value: 'tabs',
      label: 'Tab Strip',
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
      value: 'lengths',
      label: 'Lengths',
    },
    {
      value: 'changes',
      label: 'Changes',
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
      value: 'panel',
      label: 'Panel',
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
  ],
  '/dialogs': [
    { value: 'account', label: 'Account' },
    { value: 'providers', label: 'Providers' },
    { value: 'devices', label: 'Devices' },
    { value: 'workspace', label: 'Workspace' },
    { value: 'cloud', label: 'Cloud' },
    { value: 'conversation', label: 'Conversation' },
    { value: 'skills', label: 'Skills' },
  ],
  '/markdown': [
    {
      value: 'markdown',
      label: 'Markdown',
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
