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
in the prototype remains browser-session fixture state. The backend also persists
ordering independently; frontend wiring to it is pending.

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

Project dragging temporarily folds every project without modifying stored fold
preferences. Drop or cancellation restores those preferences, then reveals the
dragged project with smooth scrolling toward the viewport’s vertical center after
layout transitions finish. Scroll limits clamp positioning near either end of the list. Reduced-motion
preference uses instant positioning. Targets
are refreshed each animation frame as headers move. Conversation drag is restricted to its existing project and pin partition; reordering cannot
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
- `packages/backend/src/__tests__/state.test.ts`: project order is independent
  from recently used directory history.
- Browser checks: pointer reorder for conversations and projects, Alt+Down,
  long-list scrolling and visible scrollbar. The fixture includes 40 conversations
  across ordinary conversations, demi and notes, including pinned and long names.


## Backend persistence

`storage/control.ts` stores conversation pin and sort positions and project sort
positions in control.sqlite. New conversations enter at the front; new projects
append. Renaming and inference activity keep positions. Target changes and
archive/restore retain the saved position; equal positions after group changes
use the stable conversation ID as the tie-breaker. Pin changes retain the position
within the new partition. The API returns the resulting order, so callers do not
need to reproduce the comparison rules.

`POST /api/sidebar/reorder` persists one insertion atomically and rejects targets
outside the caller's existing group and pin partition. Archived rows cannot be
reordered. Tests in `packages/backend/src/__tests__/sidebar-backend.test.ts` cover
persistence across restart, pin partition rejection and activity independence.
