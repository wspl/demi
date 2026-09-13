export interface EditorLspCapabilities {
  completionProvider?: { triggerCharacters?: string[]; resolveProvider?: boolean } | null
  definitionProvider?: boolean | Record<string, unknown> | null
  hoverProvider?: boolean | Record<string, unknown> | null
  signatureHelpProvider?: { triggerCharacters?: string[] } | null
  foldingRangeProvider?: boolean | Record<string, unknown> | null
  inlayHintProvider?: boolean | Record<string, unknown> | null
  semanticTokensProvider?: {
    full?: boolean
    legend: {
      tokenTypes: string[]
    }
  } | null
}

export interface EditorDiagnostic {
  range: {
    start: { line: number; character: number }
    end: { line: number; character: number }
  }
  severity?: number
  message: string
  source?: string
}

export interface EditorLspTarget {
  hostId: string
  cwd: string
  pluginId: string
  capabilities: EditorLspCapabilities
}

export interface EditorHost {
  theme: {
    getSnapshot(): { mode: 'light' | 'dark'; codeThemeId: string; palette: unknown }
    subscribe(listener: () => void): () => void
  }
  lsp: {
    getTargetsForResource(resourceUri: string): EditorLspTarget[]
    toDocumentUri(target: EditorLspTarget, resourceUri: string): string
    fromDocumentUri(target: EditorLspTarget, documentUri: string, contextResourceUri?: string): string | null
    getLanguageId(resourceUri: string): string
    sendRequest(target: Pick<EditorLspTarget, 'hostId' | 'cwd' | 'pluginId'>, method: string, params: unknown): Promise<unknown>
    sendNotification(target: Pick<EditorLspTarget, 'hostId' | 'cwd' | 'pluginId'>, method: string, params: unknown): void
    getDiagnostics(resourceUri: string): EditorDiagnostic[]
    subscribeWorkdirs(listener: () => void): () => void
  }
  i18n: {
    t(key: string): string
  }
  markdown: {
    render(src: string, options?: Record<string, unknown>): string
  }
  navigation?: {
    openResource(
      resourceUri: string,
      options?: {
        selection?: {
          start: { line: number; character: number }
          end: { line: number; character: number }
        }
        reveal?: boolean
      },
    ): Promise<void> | void
  }
}
