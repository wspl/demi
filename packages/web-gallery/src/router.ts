import { createRouter, createWebHistory } from 'vue-router'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'
import OverviewSection from './sections/OverviewSection.vue'
import SignInSection from './sections/SignInSection.vue'
import SurfacesSection from './sections/SurfacesSection.vue'
import PrimitivesSection from './sections/PrimitivesSection.vue'
import ControlLayoutSection from './sections/ControlLayoutSection.vue'
import WritingSection from './sections/WritingSection.vue'
import ErrorsSection from './sections/ErrorsSection.vue'
import MotionSection from './sections/MotionSection.vue'
import OverlaysSection from './sections/OverlaysSection.vue'
import DialogsSection from './sections/DialogsSection.vue'
import FilesSection from './sections/FilesSection.vue'
import SessionSection from './sections/SessionSection.vue'
import SidebarSection from './sections/SidebarSection.vue'
import SettingsSection from './sections/SettingsSection.vue'
import MarkdownSection from './sections/MarkdownSection.vue'
import RoadmapSection from './sections/RoadmapSection.vue'
import PhoneSection from './sections/PhoneSection.vue'
import PhoneFrameView from './phone/PhoneFrameView.vue'
import AutomationsSection from './sections/AutomationsSection.vue'
import AutomationsFrameView from './phone/AutomationsFrameView.vue'
import { t } from './locales/text'

/** `frame`: a page alone, with no gallery around it, as a phone frame shows it. */
export type GalleryLayout = 'catalog' | 'session' | 'preview' | 'frame'

/** The navigation's pages, named in the page's language each time they are read. */
export function galleryNav(): { path: string; label: TitleText }[] {
  return [
    { path: '/overview', label: t('nav.overview') },
    { path: '/signin', label: t('nav.signin') },
    { path: '/surfaces', label: t('nav.surfaces') },
    { path: '/primitives', label: t('nav.primitives') },
    { path: '/control-layout', label: t('nav.controlLayout') },
    { path: '/writing', label: t('nav.writing') },
    { path: '/errors', label: t('nav.errors') },
    { path: '/motion', label: t('nav.motion') },
    { path: '/overlays', label: t('nav.overlays') },
    { path: '/dialogs', label: t('nav.dialogs') },
    { path: '/files', label: t('nav.files') },
    { path: '/session', label: t('nav.session') },
    { path: '/sidebar', label: t('nav.sidebar') },
    { path: '/automations', label: t('nav.automations') },
    { path: '/settings', label: t('nav.settings') },
    { path: '/markdown', label: t('nav.markdown') },
    { path: '/phone', label: t('nav.phone') },
    { path: '/roadmap', label: t('nav.roadmap') },
  ]
}

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/overview' },
    {
      path: '/overview',
      component: OverviewSection,
      meta: { layout: 'catalog' }
    },
    { path: '/signin', component: SignInSection, meta: { layout: 'catalog' } },
    {
      path: '/surfaces',
      component: SurfacesSection,
      meta: { layout: 'catalog' }
    },
    {
      path: '/primitives',
      component: PrimitivesSection,
      meta: { layout: 'catalog' }
    },
    { path: '/control-layout', component: ControlLayoutSection, meta: { layout: 'catalog' } },
    { path: '/writing', component: WritingSection, meta: { layout: 'catalog' } },
    { path: '/errors', component: ErrorsSection, meta: { layout: 'catalog' } },
    { path: '/motion', component: MotionSection, meta: { layout: 'catalog' } },
    {
      path: '/overlays',
      component: OverlaysSection,
      meta: { layout: 'catalog' }
    },
    { path: '/dialogs', component: DialogsSection, meta: { layout: 'catalog' } },
    { path: '/files', component: FilesSection, meta: { layout: 'catalog' } },
    { path: '/session', component: SessionSection, meta: { layout: 'session' } },
    { path: '/sidebar', component: SidebarSection, meta: { layout: 'catalog' } },
    { path: '/automations', component: AutomationsSection, meta: { layout: 'catalog' } },
    {
      path: '/settings',
      component: SettingsSection,
      meta: { layout: 'catalog' }
    },
    {
      path: '/markdown',
      component: MarkdownSection,
      meta: { layout: 'preview' }
    },
    { path: '/roadmap', component: RoadmapSection, meta: { layout: 'catalog' } },
    { path: '/phone', component: PhoneSection, meta: { layout: 'catalog' } },
    { path: '/phone-frame', component: PhoneFrameView, meta: { layout: 'frame' } },
    { path: '/automations-frame', component: AutomationsFrameView, meta: { layout: 'frame' } },
    { path: '/:pathMatch(.*)*', redirect: '/overview' },
  ],
})

declare module 'vue-router' {
  interface RouteMeta {
    layout?: GalleryLayout
  }
}
