---
status: proposed
---

# A tab's menu closes every tab

## Problem

A work panel holds a file, two web pages and a change the agent opened. To
start clean, the user closes them one by one, or chooses Close Others on one
tab and then closes that tab too: two steps for one wish.

## Decision

A tab's menu ends with Close All, which closes every tab the panel opened.
The pinned tabs, such as Changes and Files, stay, since they never close; the
panel stays open and shows the pinned tab selected last, or the first one.

```text
  Copy Address            <- the tab's own commands
  ----------------------
  Close
  Close to the Right
  Close to the Left
  Close Others            <- moved down from second
  Close All               <- new
```

Close Others moves down to stand above Close All, so the commands that close
around the tab come first and the two that sweep the strip close the menu.
Close All is always enabled, as the tab it is chosen on is one to close. It
asks nothing first, as Close Others does not, and the closed tabs leave as one
request, the way Close Others leaves.

`TabCloseScope` gains `'all'`, and `tabsToClose` returns every tab's id for
it. The labels are English *Close All*, Simplified Chinese 全部关闭,
Traditional Chinese 全部關閉, Japanese すべて閉じる and Korean 모두 닫기.

## Spike

None.

## Prior art

VS Code's editor tab menu
([`editor.contribution.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/editor/editor.contribution.ts),
`MenuId.EditorTitleContext`) has Close, Close Others, Close to the Right, Close
Saved and Close All; Demi's menu already follows its names, so Close All
takes its name and its place at the end. Demi departs from its order by
keeping Close Others beside Close All, the two commands that close more than
one side. Safari and Chrome have no
Close All in a tab's menu, where closing every tab would close the window;
here the pinned tabs keep the panel.

## Alternatives

- Close All in the strip's list of tabs: the list chooses a tab to show, and a
  command there is out of place; the tab menu is where Close Others already is.
- Close All closing the panel too: the panel's own button closes it, and a
  command named for tabs should not hide the panel.

## Open questions

None.
