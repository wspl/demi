import type { EditorHost } from './types'
import { inferLanguageIdFromResourceUri } from '../language/languageId'

export function createEditorHostDefaults(): EditorHost {
  return {
    theme: {
      getSnapshot: () => ({ mode: 'dark', codeThemeId: 'one', palette: null }),
      subscribe: () => () => {},
    },
    lsp: {
      getTargetsForResource: () => [],
      toDocumentUri: (_target, resourceUri) => resourceUri,
      fromDocumentUri: (_target, documentUri) => documentUri,
      getLanguageId: inferLanguageIdFromResourceUri,
      sendRequest: async () => null,
      sendNotification: () => {},
      getDiagnostics: () => [],
      subscribeWorkdirs: () => () => {},
    },
    i18n: {
      t: (key: string) => key,
    },
    markdown: {
      render: (src: string) => src,
    },
    navigation: {
      openResource: () => {},
    },
  }
}
