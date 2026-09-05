# Sidebar ordering and motion

The shared sidebar renders section headings, project headers and visible
conversations in one keyed TransitionGroup. Insertion, archive/delete, pin changes,
folding and reordering move the remaining entries over 180ms. Leaving entries are
removed from layout while fading; reduced-motion preference disables transitions.
The scroll container uses a visible thin thumb and a stable scrollbar gutter.

Array order is the ordering contract. Projects follow the resource array.
Conversations keep their supplied array order within each project, with pinned
entries partitioned first. Activity timestamps do not change manual placement.
A new conversation is inserted at the front of the unpinned partition. Ordering
lives in the prototype stores for the current browser session, like other
prototype state; reload restores the fixtures.

```text
Zan drags “First draft” in notes

pointer down on title
    | move at least 5px
    v
floating title + insertion line; real order stays intact
    | pointer approaches scroll-container edge
    +--> scroll list and recompute the insertion target
    |
    +-- release over a valid notes/unpinned row
    |      --> emit {kind: conversation, id: draft, beforeId: writing}
    |      --> conversation store reorders its array
    |      --> keyed rows animate to their new positions
    |
    +-- Escape / pointer cancellation / release outside a valid target
           --> discard target; order is unchanged
```

Project drag moves the project and its visible children together. Conversation
drag is restricted to its existing project and pin partition; reordering cannot
change the execution environment. Binding changes use the existing project/host
controls. Dragging one row moves that row, independently of multi-selection.
Pin/archive/create controls and rename fields do not initiate dragging. Ordinary
clicks still select, open or fold; finishing a drag suppresses the synthetic click.
Alt+Up/Down reorders the keyboard-focused entry within the same allowed partition.

`sidebar/reorder.ts` resolves valid peers and insertion requests.
`sidebar/useSidebarDrag.ts` owns pointer interaction, cancellation, drag feedback
and edge scrolling. AppSidebar emits the request without mutating product state.
Product stores and the live gallery apply array moves using the generic
`moveBefore` utility from `@demicodes/utils`.

## Verification

- `packages/utils/src/reorder.test.ts`: non-mutating moves in both directions,
  append, self moves and invalid item/target handling.
- `packages/web-ui/src/sidebar/__tests__/reorder.test.ts`: before/after/end drop
  resolution, no-op drops, project order and group/pin boundaries.
- `packages/web-ui/src/sidebar/__tests__/group-conversations.test.ts`: stable
  project order and manual conversation order despite activity changes.
- `packages/web/src/conversation/store.test.ts`: stored order survives a turn;
  cross-project reorder requests are rejected.
- `packages/web/src/prototype/resources.test.ts`: project order is independent
  from recently used directory history.
- Browser checks: pointer reorder for conversations and projects, Alt+Down,
  long-list scrolling and visible scrollbar. The fixture includes 40 conversations
  across ordinary conversations, demi and notes, including pinned and long names.
