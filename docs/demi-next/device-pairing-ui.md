# Device pairing UI

The reusable `web-ui/devices/DevicePairingDialog` and `useDevicePairing` own the
pairing presentation and lifecycle. Settings, onboarding and host pickers can
supply installer endpoints, the backend URL and an asynchronous claim adapter. The current product and
gallery use prototype adapters, not live device APIs.

Devices has an Add device button on the right of Your devices. Revocation is
reflected by removal from the list; it does not append success text. A blocked
revocation uses an error toast because the action has no input field.

## Flow

1. Install and start runner: select Linux, macOS or Windows. A bordered command
   box displays a curl-to-shell command on Linux/macOS, or a PowerShell `.ps1`
   invocation on Windows. Copy always uses the selected system's command.
   The deployment supplies `DeviceInstallation` (shell and PowerShell installer
   URLs plus backend URL); `deviceInstallCommand` owns shell-specific quoting.
   The proposed scripts install/start the runner with the supplied backend.
   Keep the terminal open until pairing completes.
   These installers are not implemented or published in this repository yet;
   OS selection is a UI prototype, not a claim of native Windows runner support.
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
The gallery uses reserved example-domain installer URLs and performs no connection.
The product prototype uses same-origin placeholder installer paths; a production
host must supply published scripts before offering executable setup commands.
