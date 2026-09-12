/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** The local development account, from the repository's `.env`; development builds only. */
  readonly DEMI_DEV_EMAIL?: string
  readonly DEMI_DEV_PASSWORD?: string
}

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, unknown>, Record<string, unknown>, unknown>
  export default component
}
