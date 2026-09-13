# Device pairing

Pairing links a running user-device runner to the signed-in account. The runner
connects first and receives a pending code from the backend. The browser claims
that code; it never creates a code or receives the device's authentication token.
Managed Cloud devices receive pre-issued tokens and do not use this flow.

`web-ui/devices/DevicePairingDialog` owns presentation and
`useDevicePairing` owns its lifecycle. Product and gallery supply a
`DeviceInstallation` and an asynchronous claim adapter. The product adapter
calls the real `/api/devices/claim` endpoint and refreshes account state. The
gallery uses fixture results and reserved example-domain installer URLs.

```text
Runner -- unauthenticated connection --> backend -- pending code --> runner log
Browser -- signed-in claim(code) ------> backend -- device token --> runner
                                        |
                                        +-- device identity --> browser list
```

## Install and claim

1. Add device opens setup with Linux, macOS, and Windows choices. `CopyCode`
   shows and copies the selected command. `deviceInstallCommand` quotes the
   supplied URL for the selected shell: curl piped to `sh` on Unix, or
   `irm '<installer URL>' | iex` in PowerShell.
2. The deployment's script downloads and starts its runner. The user reads the
   pending code from runner output or its pairing log and continues to code entry.
3. The user pastes the complete code. The UI trims surrounding whitespace; the
   backend owns normalization and verification.
4. Submission changes the phase from `code` to `pairing`, retains the visible
   input, and blocks duplicate submission.
5. Success shows the returned device name and Done. The refreshed device list
   shows the linked device; it does not retain an additional success message.

The shared phase is `setup | code | pairing | done`; an error belongs to the
code-entry phase so the user can correct or retry it. Back returns to setup.
Opening the dialog starts a new setup flow. Entry points in Settings, the host
menu, and the workspace form use this same interaction; a nested entry stacks
the pairing dialog above its parent.

## Failure and closure

| Result | UI behavior |
|---|---|
| `invalid_code` | Ask for the latest code from the running runner |
| `rate_limited` | Ask the user to wait before retrying |
| Network or other unavailable result | Keep input and offer another attempt |

Invalid, expired, already-used, and disconnected codes share `invalid_code`.
The UI does not invent a more specific cause or a countdown. The registry rotates
pending codes every ten minutes by default and limits claim attempts per user.
[Runner](runner.md) owns connection and registration semantics.

Closing, reopening, resetting, or unmounting aborts the outstanding browser
request and invalidates its UI generation. Late results cannot reopen or advance
an old dialog. Aborting HTTP does not roll back a claim already committed by the
backend: account snapshots remain authoritative for the device list. If a
response or the post-claim refresh is lost, inspect that list before assuming the
claim failed; the same one-use code may no longer be available.

Revocation is separate from pairing. Devices settings removes a successfully
revoked user device from the list. A refused revoke shows an error toast. The
backend refuses revocation while workspaces reference that device and does not
allow this action for Cloud. API details belong to [Web API](web-api.md).

## Installation boundary

The product supplies same-origin `/install.sh` and `/install.ps1` URLs. The backend
implements both routes over its configured native release directory
(`DEMI_RUNNER_RELEASE_DIR`). Scripts carry the backend address, select a platform
artifact, verify its checksum, and start a registration-specific runner. Downloads
contain no device credentials. [Native runtime](native-runtime.md) owns artifact
publication, installation isolation, and upgrade behavior.

A deployment must provide the native release manifest and artifacts for each
platform it offers. Without release configuration the script routes return 503;
a platform choice in the dialog does not prove its artifact is available.
The Unix copy command restricts curl to HTTPS, so the displayed command requires
an HTTPS deployment; it cannot bootstrap directly from the plain-HTTP development
origin.

## Implementation and acceptance limits

Live claiming and script generation are implemented. The setup copy still tells
the user to keep the terminal open, although generated installers start a
background runner. Unix prints its initial log; the Windows installer prints the
pairing-log path rather than the code itself. The setup guidance should identify
that log when the code is not visible. This is a remaining UI guidance discrepancy,
not a missing installer implementation.

The Settings gallery pins setup, code, pairing, representative error, and done
states and provides a fixture happy path. Verify input retention, duplicate-submit
blocking, closure during a request, a late success, and a nested-dialog entry.
Real acceptance also requires a configured release and an actual runner claim;
fixture success does not verify native installation on each supported OS.
