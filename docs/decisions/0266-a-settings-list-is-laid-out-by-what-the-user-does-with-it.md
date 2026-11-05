---
status: proposed
---

# A settings list is laid out by what the user does with it

## Problem

Several settings pages list things the user manages, and each lists them the
same way, a card of rows, whatever the user does with them:

- **Credentials.** A user with 150 credentials scrolls one card of
  three-line rows, with no search and no sort. A credential cannot be
  edited: to fix a wrong note or a changed password, they delete it and wait
  for an agent to ask again. Cleaning up means 40 Delete… clicks, each with
  its own confirmation.
- **Port Forwarding.** Each forward opens its own page, so changing three
  ports means opening and leaving three pages.
- **Archived.** Restoring or deleting 30 conversations is 30 rounds of
  clicks.
- **Users.** What an admin does to an account (change the role, reset the
  password, sign the account out) is behind a ··· menu, each in a dialog.
- **Several at once.** Cloud Network's addresses, a compatible API's headers
  and its models are added one by one, even when the user has them as a list
  to paste.

## Decision

### Three layouts

A page takes the layout that fits what the user does with its things:

| The page holds                                 | Layout                  | Pages                                       |
| ---------------------------------------------- | ----------------------- | ------------------------------------------- |
| Things, each with settings of its own          | A rail beside a detail  | Providers, Workspaces, Credentials, Users   |
| Things with the same few short fields          | An editable table       | Port Forwarding                             |
| Things the user mostly finds, opens or removes | A list with checkboxes  | Archived                                    |

Subagents and Skills keep the layout they have.

**A rail beside a detail** is `SettingsSplit`: the rail lists the things, and
the detail edits the selected one in place, with no dialog.

```
Credentials
178 CREDENTIALS        ⇅ +   Airtable sign-in                       [trash]
[Search             ]       ┌───────────────────────────────────────────┐
A                           │ Name   [Airtable sign-in                ] │
▌Airtable sign-in           │ Note   [To sign in to the dashboard     ] │
 ops                        └───────────────────────────────────────────┘
 Airtable token for deploys  Fields                            Add Field
 To push release branches…  ┌───────────────────────────────────────────┐
                            │ User name  [ops                         ] │
                            │ Password   [••••••••                  👁] │
                            └───────────────────────────────────────────┘
```

- A page's own settings, such as Users' choice of whose providers everyone
  uses, are the rail's first entry,
  **General Settings**, above the things' group. A bare *General* would not
  say what the entry holds beside a list of things.
- A rail of many two-line entries, such as credentials, is wider (15rem
  instead of 11rem) and keeps its search and buttons above the entries
  while they scroll.
- The entries keep their height however long the list is.
- Credentials are grouped under sticky headers: by first letter when sorted
  by name, by Today, This Week, This Month, Earlier This Year, Older and
  Never Used when sorted by time. The sorts are Name, Recently Used, Least
  Recently Used and Date Added. A credential's second line is its first
  visible field's value, else its note, else nothing.
- Users shows an account's facts and every action an admin may take on it
  as the detail's rows, instead of a menu of dialogs.

**An editable table** edits each field in its cell:

```
Your Forwards                                              + Add Forward
┌──────────────────────────────────────────────────────────────────────┐
│ ☐  Name       Listen On      Port   Connect To     Address        ◯  │
│ ☐  ● ollama   ☁ Cloud ⌄      11434   alex-mbp ⌄   localhost:11434 ●  │
│ ☐  ● postgres ☁ Cloud ⌄      5432    alex-mbp ⌄   127.0.0.1:5432  ●  │
└──────────────────────────────────────────────────────────────────────┘
```

- A text cell is a bare input that shows its field on hover and focus.
- A Host cell is a ghost dropdown: the Host's glyph, its name and a chevron
  right after it, with no fill until the pointer is over it. `Dropdown` gains
  this look as a variant for any field that should read as text until used.
- The dot is the forward's state; its words are its tooltip. A red or yellow
  dot opens a popover with the problem and Try Again. No line under a row
  says it, and no legend explains the colors.
- On a phone the table becomes rows of the dot, the name, the route on one
  line and the switch; a row opens the forward's page.

**A list with checkboxes** has one line per row, here the title, then its
project and when it was last active, to the minute, in columns at its end,
which Open and Restore replace under the pointer in the same space. The page
lists one workspace's conversations, so a row does not name the workspace.

### Selecting several

- In a rail whose things have an action that applies to several,
  Credentials, ⌘-click and Shift-click select several, as in Finder and
  Passwords; the detail then names them and offers that action, such as
  Delete 5 Credentials…. Users has none: each of its actions is about one
  account, and its question names that account.
- In a table or a list, each row has a checkbox. Once one is checked, the
  title row above the card becomes the count and the actions (Turn On, Turn
  Off, Delete… for forwards; Restore, Delete… for conversations), at the
  same height, so nothing moves.
- On a phone, which has neither ⌘ nor hover, a Select button turns the rows
  into checkboxes. Where three actions and Done do not fit the title row's
  width, the actions are one Actions menu on that row.
- An action on several is the action on one, done for each: one
  confirmation, then the page calls the existing route for each thing.

### Create first

New Credential creates the credential at once, with a free name (*New
Credential*, *New Credential (2)*), selects it and opens its detail. A field it cannot work without, such as a new
credential's password, is shown as missing. Add Forward already creates a
forward whose Connect To is missing.

### Several at once

- Cloud Network adds every address of a paste, or of a line split by commas.
- A compatible API's header name field turns a paste of several
  `Name: value`, `Name=value` or `-H 'Name: value'` lines into a row each.
- Add Model turns several model IDs into one choice, Add 3 Models.
- Signed-In Browsers offers Sign Out Other Browsers….

### The backend

Only credentials need new routes and data.

`PATCH /credentials/{id}` changes a credential:

```rust
/// `PATCH /credentials/{id}`: what changes; an absent member stays.
pub struct CredentialPatch {
    pub name: Option<CredentialName>,
    pub note: Option<String>,
    /// The fields as they now are, in order. A field that keeps its name
    /// and its kind keeps its value; a field that is new, or whose kind
    /// changed, has none until `values` gives it one.
    pub fields: Option<Vec<CredentialField>>,
    /// A new value for each field named, a secret one included. A value is
    /// written, never read: no route answers a secret's value.
    pub values: BTreeMap<String, String>,
    pub keep: Option<Keep>,
}
```

- A field may have no value. The list says so (`ListedField` gains
  `missing: bool`), and an agent that reads it is refused in words that name
  the field and say the user fills it in Settings › Credentials.
- `POST /credentials` accepts a field with an empty value as missing, so New
  Credential can create one before its secret is typed.
- The page's credential gains `createdAt`, which `CredentialDto` already
  carries, for Date Added.
- A change reaches every page as the `credentials` part, as a creation
  does.

## Spike

None: every layout above was built as a gallery concept in the real
settings dialog, at desktop and phone widths, before this record.

## Prior art

- macOS Passwords and Finder: a searchable rail beside a detail, with ⌘ and
  Shift to select several.
- macOS System Settings' DNS servers and the `ssh -L` tables of tools such as
  VS Code's Ports view: short, uniform fields edited in place.
- Gmail and GitHub: checkboxes on a full-width list, with the actions on
  the checked rows in the bar above it.

## Alternatives

- **One list layout everywhere.** Modern settings turn tables into lists
  that open each row, which costs a round trip for every change of a short
  field and leaves no room to select several.
- **A drill-down page per row for Users**, with the page's own
  settings above the list. It hides the detail behind a click, and the
  rail's General Settings entry keeps both on one page.
- **A website field on credentials.** It would need a column the data has no
  use for yet.
- **A batch route per action.** The page calls each thing's route, which
  the user's lists are short enough for; a route can come when one is not.

## Open questions

None.
