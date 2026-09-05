import type { FileExtension, ModelSelection } from '@demicodes/core'
import type { Conversation, Device, Project, PrototypeProvider } from './types'

export function providers(): PrototypeProvider[] {
  return [
    {
      id: 'demo',
      label: 'Demo provider',
      isAvailable: true,
      models: [
        {
          id: 'balanced',
          name: 'Balanced',
          contextWindow: 200000,
          inputLimit: 180000,
          acceptedExtensions: ['png', 'jpg', 'webp', 'pdf'],
          reasoning: {
            efforts: ['low', 'medium', 'high'],
            defaultEffort: 'medium',
            canDisable: true,
          },
          serviceTiers: [{ id: 'fast', label: 'Fast', fast: true }],
        },
        {
          id: 'precise',
          name: 'Precise',
          contextWindow: 200000,
          inputLimit: 180000,
          acceptedExtensions: ['png', 'jpg', 'webp', 'pdf'],
          reasoning: { efforts: ['medium', 'high'], defaultEffort: 'high', canDisable: false },
          serviceTiers: null,
        },
      ],
    },
  ]
}
export function devices(): Device[] {
  return [
    { id: 'mac', name: 'zan-mbp', online: true, home: '/Users/zan' },
    { id: 'build', name: 'build-01', online: true, home: '/home/build' },
  ]
}
export function projects(): Project[] {
  return [
    {
      id: 'demi',
      name: 'demi',
      deviceId: 'mac',
      host: 'zan-mbp',
      hostKind: 'device',
      path: '/Users/zan/Projects/demi',
      branch: 'main',
    },
    {
      id: 'notes',
      name: 'notes',
      deviceId: 'mac',
      host: 'zan-mbp',
      hostKind: 'device',
      path: '/Users/zan/Projects/notes',
      branch: 'writing',
    },
  ]
}
export function conversation(
  id: string,
  title: string,
  projectId: string | null = null,
): Conversation {
  return {
    id,
    title,
    projectId,
    updatedAt: new Date().toISOString(),
    status: 'idle',
    pinned: false,
    unread: false,
    archived: false,
    blocks: [],
    draft: '',
    files: [],
    queue: [],
    providerId: 'demo',
    modelId: 'balanced',
    thinking: { type: 'effort', effort: 'medium', summary: null },
    serviceTierId: null,
    attachedHosts: [],
    stream: null,
  }
}
export function conversations(): Conversation[] {
  const c = conversation('welcome', 'A place to think and build', 'demi')
  const model = modelSelection(c)
  c.blocks = [
    {
      id: 'welcome-user',
      type: 'user',
      turnId: 'welcome-turn',
      createdAt: c.updatedAt,
      model,
      content: [{ type: 'text', text: 'Let’s make a workspace for our next idea.' }],
      preamble: null,
    },
    {
      id: 'welcome-answer',
      type: 'text',
      createdAt: c.updatedAt,
      model,
      text: 'Everything starts with a conversation.\n\nWe can shape an idea, work through a question, or build something together. Your projects keep related conversations close, and each conversation has its own working environment.\n\n**What would you like to work on?**',
    },
  ]
  return [
    c,
    conversation('ideas', 'Ideas for the week'),
    conversation('writing', 'Find a clearer way to say it', 'notes'),
    { ...conversation('reading', 'Reading list for the weekend'), pinned: true },
    conversation('week-plan', 'Plan the next week'),
    conversation('travel-notes', 'Compare a few places for a quiet weekend away'),
    { ...conversation('web-review', 'Web interface review', 'demi'), pinned: true },
    { ...conversation('release-plan', 'Release checklist', 'demi'), pinned: true },
    conversation('sidebar-motion', 'Sidebar sorting and animation details', 'demi'),
    conversation('workspace-flow', 'Review the workspace selection flow', 'demi'),
    { ...conversation('outline', 'Article outline', 'notes'), pinned: true },
    conversation('draft', 'First draft', 'notes'),
    conversation(
      'examples',
      'Collect examples that make the explanation easier to follow',
      'notes',
    ),
    ...[
      'Book recommendations',
      'Questions for the next meeting',
      'Weekend cooking ideas',
      'A better morning routine',
      'Trip packing checklist',
      'Learn something new this month',
      'Compare notes from last week',
      'Organize photos and memories',
      'Ideas worth revisiting',
    ].map((title, index) => conversation(`personal-${index}`, title)),
    ...[
      'Keyboard navigation review',
      'Directory picker edge cases',
      'Check sidebar spacing',
      'Review empty and error states',
      'Prepare release notes',
      'Investigate rendering performance',
      'Verify device attachment behavior',
      'Design a clearer settings page',
      'Explore message search interactions',
    ].map((title, index) => conversation(`demi-${index}`, title, 'demi')),
    ...[
      'Opening paragraph alternatives',
      'Research notes',
      'Edit the introduction',
      'Find a stronger conclusion',
      'Examples for the second chapter',
      'Review tone and consistency',
      'Interview questions',
      'Ideas for the next article',
      'A concise summary for readers',
    ].map((title, index) => conversation(`notes-${index}`, title, 'notes')),
  ]
}

export function modelSelection(c: Conversation): ModelSelection {
  const entry = providers()[0]!.models.find((model) => model.id === c.modelId)!
  return {
    providerId: c.providerId,
    model: {
      id: entry.id,
      name: entry.name,
      contextWindow: entry.contextWindow!,
      inputLimit: entry.inputLimit,
      thinking: [],
      acceptedExtensions: entry.acceptedExtensions as FileExtension[],
    },
    thinking: c.thinking,
    serviceTierId: c.serviceTierId,
  }
}
