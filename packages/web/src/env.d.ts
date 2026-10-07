/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** The local development account, from the repository's `.env`; development builds only. */
  readonly DEMI_DEV_EMAIL?: string
  readonly DEMI_DEV_PASSWORD?: string
  /** This page's build, which `vite build` writes in (`vite.config.ts`); none in development. */
  readonly DEMI_WEB_BUILD?: string
  /** The release of the preview runtime the page carries at `/runtime/<release>.js`; empty without one (`vite.config.ts`). */
  readonly DEMI_PREVIEW_RUNTIME?: string
}

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, unknown>, Record<string, unknown>, unknown>
  export default component
}
