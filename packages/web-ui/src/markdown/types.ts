export interface MarkdownThemeSnapshot {
  mode: 'light' | 'dark'
  codeThemeId: string
}

export interface MarkdownRenderOptions {
  knownPaths?: Set<string>
  basePath?: string
  theme?: MarkdownThemeSnapshot
}

export type MarkdownRenderer = (src: string, options?: MarkdownRenderOptions) => string

export interface MarkdownLinkClick {
  href: string
  event: MouseEvent
  basePath?: string
}

export type MarkdownLinkHandler = (payload: MarkdownLinkClick) => void
