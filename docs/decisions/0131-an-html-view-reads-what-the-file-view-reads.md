---
status: proposed
---

# An HTML view reads what the File view reads

Supersedes: 0119 (An HTML file shows as a page), the part that keeps a page
to the files of the workspace that are not hidden.

## Problem

An agent writes `report.html` in its scratch directory,
`/Users/alex/.demi/scratch/1a2b3c4d5e6f/`, and gives it as an attachment.
Alex opens the attachment, and the File view shows
`{"code":"not_found","message":"No such file"}` in place of the page: the
HTML route refuses a path with a name that starts with `.`, and every
scratch directory lies under `.demi`. When the working directory is a
project, such as `/Users/alex/shop`, the grant refuses the file before that,
because it is outside the working directory.

The limits guard nothing the user does not already reach: the File view
reads any path on the Host, a hidden one or one outside the workspace
included, by its absolute path.

## Decision

An HTML view loads any file the File view can read, wherever it is on the
Host.

- **The grant** answers the file's absolute path, its names from the root
  joined by `/`: `/api/html/<token>/Users/alex/.demi/scratch/1a2b3c4d5e6f/report.html`.
  A Windows path keeps its drive or share as its first names,
  `/api/html/<token>/C%3A/Users/alex/report.html`. A `:` in a name is
  encoded, so a folder's redirect to `a:b/` is not read as a scheme.
  `HostPaths::place` and `HostPaths::at_place` in `host-interface` turn a
  path into its place and back, in the Host's form of paths.
- **The route** reads the path the place names, as the File view reads one,
  with no check of hidden names, of the working directory or of where a
  symbolic link leads. A place above a Windows drive's root, which a
  relative URL that climbs too far asks for, names no path and answers 404.
- A page's relative URLs reach every file on the Host, so a hostile page
  could send any of them to a site of its choosing, not only the
  workspace's. The sandbox, the token, the headers and the media types stay
  as 0119 has them.
- An older page asks for the grant as before and loads the URL it answers.
  Nothing is stored, and no runner changes.

## Spike

None.

## Prior art

Jupyter Server serves the files under its root directory and refuses hidden
ones unless `allow_hidden` is set, because its file browser keeps to that
root too. Demi's File view keeps to no root, so its HTML view keeps to none
either.

## Alternatives

- Keep the limits and add the conversation's scratch directory as a second
  root: a page keeps a limit the File view lacks, and a file outside both
  still shows as a 404.
- Read only the web's file types, as an allowlist: a page's data can be a
  JSON file, and a credential can be one too, so the list neither keeps
  secrets out nor lets every page work.

## Open questions

None.
