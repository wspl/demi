import { createRouter, createWebHistory } from 'vue-router'
import OverviewSection from './sections/OverviewSection.vue'
import SurfacesSection from './sections/SurfacesSection.vue'
import PrimitivesSection from './sections/PrimitivesSection.vue'
import MotionSection from './sections/MotionSection.vue'
import OverlaysSection from './sections/OverlaysSection.vue'
import SessionSection from './sections/SessionSection.vue'
import SidebarSection from './sections/SidebarSection.vue'
import MarkdownSection from './sections/MarkdownSection.vue'
import CodeSection from './sections/CodeSection.vue'
import RoadmapSection from './sections/RoadmapSection.vue'
import ColorReviewSection from './sections/ColorReviewSection.vue'

export type GalleryLayout = 'catalog' | 'session' | 'preview'

export const NAV: { path: string; label: string }[] = [
  { path: '/overview', label: 'Overview' },
  { path: '/surfaces', label: 'Surfaces' },
  { path: '/primitives', label: 'Primitives' },
  { path: '/motion', label: 'Motion' },
  { path: '/overlays', label: 'Overlays' },
  { path: '/session', label: 'Session' },
  { path: '/sidebar', label: 'Sidebar' },
  { path: '/markdown', label: 'Markdown' },
  { path: '/code', label: 'Code' },
  { path: '/roadmap', label: 'Roadmap' },
  { path: '/color-review', label: 'Color review' },
]

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/overview' },
    { path: '/overview', component: OverviewSection, meta: { layout: 'catalog' } },
    { path: '/surfaces', component: SurfacesSection, meta: { layout: 'catalog' } },
    { path: '/primitives', component: PrimitivesSection, meta: { layout: 'catalog' } },
    { path: '/motion', component: MotionSection, meta: { layout: 'catalog' } },
    { path: '/overlays', component: OverlaysSection, meta: { layout: 'catalog' } },
    { path: '/session', component: SessionSection, meta: { layout: 'session' } },
    { path: '/sidebar', component: SidebarSection, meta: { layout: 'catalog' } },
    { path: '/markdown', component: MarkdownSection, meta: { layout: 'preview' } },
    { path: '/code', component: CodeSection, meta: { layout: 'preview' } },
    { path: '/roadmap', component: RoadmapSection, meta: { layout: 'catalog' } },
    { path: '/color-review', component: ColorReviewSection, meta: { layout: 'catalog' } },
    { path: '/:pathMatch(.*)*', redirect: '/overview' },
  ],
})

declare module 'vue-router' {
  interface RouteMeta {
    layout?: GalleryLayout
  }
}
