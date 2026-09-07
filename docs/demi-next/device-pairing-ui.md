# Device pairing UI

The reusable `web-ui/devices/DevicePairingDialog` and `useDevicePairing` own the
pairing presentation and lifecycle. Settings, onboarding and host pickers can
supply the same command and asynchronous claim adapter. The current product and
gallery use prototype adapters, not live device APIs.

Devices has an Add device button on the right of Your devices. Revocation is
reflected by removal from the list; it does not append success text. A blocked
revocation uses an error toast because the action has no input field.

## Flow

1. Start runner: open a terminal on the target computer, or SSH into a remote or
   headless server. Use an installed `demi-runner` and the host-provided command
   `demi-runner run --backend <deployment URL>`. Keep the process running. Already
   paired runners reconnect automatically. A binary installation/download URL
   is not defined by the current runner design; the UI does not invent one.
2. Enter code: paste the full code printed by the runner. The browser does not
   issue a device code. The backend owns normalization and verification.
3. Pairing: disable duplicate submission and keep the entered code visible.
4. Complete: display the returned device name and Done. The host adds the device
   to its list; no success notice is left below the list.

Invalid, expired, used and disconnected codes share `invalid_code` in the real
backend. The dialog asks for the runner's latest code without pretending to know
which cause occurred. `rate_limited` asks the user to wait before retrying;
network failures retain the input for retry. Closing the dialog invalidates late
UI transitions; a completed host mutation still belongs in the device list.

The backend rotates pending codes every ten minutes by default, so no countdown
is invented in the browser. Managed hosts receive pre-issued tokens and never
enter this flow. See [runner.md](runner.md) and [managed-hosts.md](managed-hosts.md).

The Settings gallery has a separate Add device pairing-flow specimen using the
same dialog, with directly selectable setup, code, pairing, invalid-code,
rate-limit, connection-error and completed states, plus a clickable happy path.
The example command uses a reserved example domain and performs no connection.
