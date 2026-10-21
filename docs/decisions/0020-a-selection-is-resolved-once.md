---
status: accepted
---

# A selection is resolved once

## Context and Problem Statement

Selecting text in an agent's reply shows a toolbar with Comment and Ask over
the selection. Three parts of the page each read the browser's selection on
their own and judge it by its DOM boundaries: the toolbar finds the message
the selection starts in and checks that it also contains the end, the
comment tools read the selection again and require both ends inside the
reply's text, and they reject any selection that touches an Ask card. The
toolbar then places itself by the selection's first rectangle.

A browser often puts a boundary where no text is selected: a triple-click
on a reply's last paragraph ends the selection at the start of the
timestamp under it, and on a paragraph before an Ask card at the start of
the card's title. The toolbar accepts such a selection and the comment tools
reject it, so the toolbar shows nothing. The first rectangle is a single run
of the first line, such as a bold word or a link, so the toolbar centers on
that run rather than on the selection, by up to a few hundred pixels.

## Considered Options

* Fix each boundary case where it fails.
* One owner resolves the selection into the reply and the text it selects,
  with their place on screen, and every part uses that.

## Decision Outcome

Chosen option: one owner, because the defects come from three readings of
one fact that disagree, and a fourth case would fail the same way.

* When the selection changes, one resolver decides which reply owns it and
  which of the reply's own text it selects, from the characters actually
  selected: a boundary that selects no character of a timestamp, a control
  or a card does not count, and a selection that takes text of another
  object does.
* The toolbar, Comment and Ask take the resolved selection; none reads the
  browser's selection itself.
* The toolbar centers on the bounds of the selected text's lines, above the
  selection on a desktop and below it on a phone, as it does now.

```text
  A reply whose second line starts with a bold word:

    The build failed because the cache was cold.
    **Restore** the cache before the first step, then run
    the tests again.

  Selecting the last two lines puts the toolbar over their middle:

                     [ Comment ]  [ Ask ]
    **Restore** the cache before the first step, then run
    the tests again.
```

### Consequences

* Good, because a selection of a reply's text shows the toolbar however the
  browser places its boundaries, and the toolbar stands over the selection's
  middle.
* Bad, because the three parts change at once to take the resolved
  selection.
