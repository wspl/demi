---
status: accepted
---

# The page reads provider facts from the backend

## Problem

The backend decides what a provider family is called and which media shapes
a compatible entry may choose, and the page keeps its own copy of each:

- Settings › Providers lists every subscription family nobody has signed in
  to yet, such as Grok Build, by a name from the page's own table
  (`subscriptionNames` in `packages/web/src/state/catalog.ts`). The backend
  names the same family with `family_name`. A family the backend adds shows
  its raw id, `grok-build`, until someone edits the page.
- A compatible entry's Images, Videos and Audio menus offer the shapes in
  the page's `COMPATIBLE_MEDIA_SHAPES`; the backend accepts the ones in each
  shape's `CHOSEN`. When the two differ, the page offers a shape the backend
  refuses, or hides one it accepts.

Two smaller copies live in the page alone:

- `wireApi` in `catalog.ts` and `protocol` in `settings/providers.ts` map a
  protocol to its family and wire in opposite directions, two tables of one
  fact.
- `PLAN_IN_DETAIL` reads Claude Code's plan from an account's detail. Claude
  Code has written no detail since its CLI owns the sign-in, and its usage
  answer names the plan. Only an account signed in before then still holds
  one, and its plan shows from it only until its usage is first read.

## Decision

What the user sees does not change. The page takes these facts from the
backend:

- `GET /providers/catalog` answers each subscription family with its name:

  ```rust
  pub struct SubscriptionFamily {
      pub provider_type: String,
      /// Its name in Settings, such as `Grok Build`.
      pub name: String,
  }
  ```

- The same answer lists the shapes a compatible entry may choose for each
  kind, from `ImageShape::CHOSEN`, `VideoShape::CHOSEN` and
  `AudioShape::CHOSEN`, in their order:

  ```rust
  pub struct VendorCatalog {
      pub subscriptions: Vec<SubscriptionFamily>,
      pub vendors: Vec<Vendor>,
      /// The shapes a compatible entry may choose for each kind of media.
      pub compatible_shapes: CompatibleShapes,
  }

  pub struct CompatibleShapes {
      pub image: Vec<ImageShape>,
      pub video: Vec<VideoShape>,
      pub audio: Vec<AudioShape>,
  }
  ```

The page drops `subscriptionNames` and `COMPATIBLE_MEDIA_SHAPES`, keeps one
table of protocols that both directions read, and drops `PLAN_IN_DETAIL`:
an account's plan is its quota snapshot's. `titleCase`, which only that rule
called, goes with it. A Claude Code account signed in before its CLI owned
the sign-in, whose usage was never read since, shows no plan until it is;
every other account shows the plan it showed.

Until the catalog is read, and while it fails, a compatible entry's
Images, Videos and Audio menus are disabled, showing the shape the entry
has; the Add dialog says the catalog's state and retries it.

The page keeps the shapes' display names (`MEDIA_SHAPE_LABELS`) and which
vendor's mark each entry wears (`vendor-logos.ts`): they are the page's text
and its own images, which the backend's names for tool messages and its
prices' vendor do not decide.

The page reads the catalog once per backend version, so a page of an
earlier version meets the new fields only after it reloads; an older page
ignores them.

## Spike

None: each copy and its readers were read in the code. The writers of an
account's detail are Codex's (`chatgpt`) and Grok Build's (`oidc`); Claude
Code's sign-in has written none since the CLI owns it.

## Prior art

None needed: the page already reads every other fact of an entry from
`GET /providers` and the catalog.

## Alternatives

- **A protocol field on the entry.** The backend would name the protocol,
  and the page would not combine family and wire. The patch would then take
  it too, which changes how every entry is edited; one table in the page
  removes the copy without that.
- **The shapes' names from the backend.** The backend's are worded for a
  call's message, such as `Lyria takes no --duration`, and differ from the
  settings menu's; the menu's are the page's text.

## Open questions

None.
