---
status: accepted
---

# A picture's long press on an iPhone is the system's, and a download from the Home Screen goes to the share sheet

## Problem

Alex uses Demi from the Home Screen of an iPhone. An agent's reply shows a
picture it made, and Alex wants it in Photos.

* **In the reply**, a long press shows Safari's menu for a link, with a
  preview of the picture as a web page: Open, Add to Reading List, Save to
  Photos, Copy, Share. The tile is a link to the picture's bytes, so the
  system treats it as a link, not as a picture.
* **In the viewer**, a long press first lifts the picture, the system's own
  preview. Half a second in, Demi's menu, Copy Image and Save Image, opens
  as a sheet, and the system's menu opens over it a moment later: Share,
  Save to Photos, Copy, Copy Subject, Look Up. Two menus offer the same
  things.
* **Demi's Save Image**, and the viewer's Download, give the picture to the
  browser as a download. Safari in a tab saves it to Files, not to Photos.
  Demi on the Home Screen shows the file in place of the whole app, with no
  way back: Alex has to close Demi and open it again. Every download does
  this there, a file card's and the Files tab's included.

Safari on iOS and iPadOS gives a page no `contextmenu` event for a long
press, and no way to stop its own menu for a picture other than
`-webkit-touch-callout: none`, which removes that menu altogether.

## Decision

On an iPhone and an iPad, a picture's long press is the system's, as a
message's text already is: Demi opens no menu of its own over it.

```text
long press on        today                             afterwards
───────────────────  ────────────────────────────────  ─────────────────────────
a reply's picture    Safari's link menu                the system's picture menu
the viewer's image   Demi's sheet, then the system's,  the system's picture menu
                     and the picture zooms on release
```

* **The system's menu.** Wherever the system shows its own menu for a
  picture, Demi's long press opens none. The page reads that from the
  picture itself: a picture whose `-webkit-touch-callout` is not `none` has
  the system's menu. A browser without the property, as on Android or a
  computer, keeps Demi's menu, with Copy Image, Save Image and a reply's own
  tools.
* **A reply's picture has no address there.** Where the system has a menu
  for pictures (`CSS.supports('-webkit-touch-callout', 'none')`), a reply's
  picture or video is a link to the viewer without an `href`
  (`role="link"`): a tap opens the viewer as before, and a long press shows
  the picture's menu rather than the link's. It is no button, whose long
  press a phone gives the page and whose touch area covers the picture. A
  computer keeps the address, so a middle click still opens the picture in a
  tab.
* **The viewer** takes the release of a long press for no tap: the picture
  stays at its size under the system's menu, where it used to zoom to its
  actual size as the finger lifted.
* **A tool's picture**, a 20-pixel thumbnail in its row, is a button whose
  touch area reaches past the picture, so the system finds no picture under
  the finger. Its long press opens the viewer, as a tap does, as today, and
  the viewer's picture has the system's menu.
* **A download from the Home Screen** goes to the share sheet. In Demi added
  to the Home Screen (`navigator.standalone`), every download, Save Image
  and Download alike, fetches the file and hands it to the share sheet
  (`navigator.share({ files })`), which offers Save Image for a picture and
  Save to Files for any file. Safari in a tab keeps the browser's download.
  * The share sheet opens only within a few seconds of the press. A file
    that takes longer to arrive, as a large one on a slow network, shows a
    toast, **Ready to Save** with the file's name, whose **Save…** opens the
    sheet with that press.
  * Closing the share sheet saves nothing and says nothing.

The design to build from:

* `packages/web-ui/src/ui/context-menu.ts`: a long press on a picture with
  the system's menu opens none.
* `packages/web-ui/src/ui/system-picture-menu.ts`: whether the system has a
  picture menu, which the two below read.
* `packages/web-ui/src/markdown/render.ts`: a medium's tile is
  `<a role="link" tabindex="0">` without `href` where the system has a
  picture menu; Enter and Space open it, as an agent's link.
* `packages/web-ui/src/files/ImagePreview.vue`: a touch held as long as a
  long press is no tap.
* `packages/web-ui/src/files/download.ts`: `downloadUrl` shares the file on
  the Home Screen.

## Spike

* **Which tile shows the picture's menu.** On iOS 26.5's Simulator, a long
  press on a picture inside a link shows the link's menu, also with
  `-webkit-touch-callout: none` on the link and `default` on the picture. A
  picture inside an element that is not a link shows the picture's menu,
  unless a phone's button rules cover it: their touch area, a pseudo-element
  over the picture, takes the press, with `-webkit-touch-callout: none`.
* **How long after a press the share sheet opens.** In Safari on the same
  Simulator, `navigator.share` with a fetched file opened the sheet when it
  was called at once and four seconds after the press, and failed with
  `NotAllowedError` eight seconds after it.

## Prior art

* iOS Photos and Messages give a picture's long press the system's menu,
  with Save to Photos, Copy and Share; so does every web page in Safari.
* [Coder](https://github.com/coder/coder/pull/27853) met the same trap: a
  download in its iOS Home Screen app covered the app with no way back. In
  the Home Screen app it shares the fetched file instead, keeps a closed
  share sheet silent, and, when the fetch outlasted the press, shows a toast
  whose Save shares it with a new press. Demi does the same.

## Alternatives

* **Demi's menu everywhere**, the system's removed with
  `-webkit-touch-callout: none`, Save Image going through the share sheet:
  one menu on every system, with a reply's own tools for a picture, but a
  picture takes the share sheet's extra step to reach Photos, where the
  system's menu has Save to Photos at once.
* **Both menus, as today**: two menus open over each other.

## Open questions

None.
