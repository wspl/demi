import type { DisplayedUserContent, DisplayedToolContent } from '@demicodes/agent/client'

/** Media accepted by the shared renderer after the host validates its transcript. */
export type DisplayedMediaSource = Extract<
  DisplayedUserContent | DisplayedToolContent,
  { type: 'image' | 'video' | 'document' }
>['source']
