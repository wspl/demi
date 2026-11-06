---
status: proposed
---

# A server runs from a container image, with one-click setups for Railway, Fly and Docker Compose

## Problem

Alex wants a Demi server and has no VPS: the only way to set one up today is
`install.sh` on a Linux machine with systemd and root
([Installation](../delivery/installation.md)). Railway and Fly run
containers, and since Simple Isolation (0268) Demi no longer needs root for
its Clouds, but nothing else is there:

- Demi publishes no container image: a release is two archives, unpacked by
  `demi-server` into `/opt/demi/releases/` (`crates/server/src/fetch.rs`).
- `demi-server` starts, stops and upgrades the two services through systemd
  (`crates/server/src/services.rs`), and the backend's requests to update are
  path units that run `demi-server upgrade --requested`. A container has no
  systemd.
- A platform's files are gone at every deploy: only a mounted volume
  outlives one, and nothing says which of Demi's directories must be on it,
  or that attachments can go to the platform's object storage instead.

So Alex needs a machine, a domain and a proxy before trying Demi at all, and
a platform that would run it for a few dollars a month is out of reach.

## Decision

Demi publishes a container image with every release, and three setups use it:

| Setup | What the operator does | Storage |
| --- | --- | --- |
| Railway | Clicks **Deploy on Railway** | A volume at `/opt/demi`, and a Railway Bucket for attachments and media |
| Fly | Pastes one block of commands | A Fly volume at `/opt/demi`, and a Tigris bucket |
| Docker Compose | `docker compose up -d` on any machine with Docker | A Docker volume at `/opt/demi`; Caddy in the same file serves HTTPS |

Inside, the server is the one `install.sh` sets up: the same layout under
`/opt/demi`, the same `demi-server`, and **Settings › Updates** updates it in
place, as on a VPS. Pulling a newer image updates it too. What a container
lacks shows as it does on any server without it: gVisor is disabled in
Settings › Cloud with the manager's reason, such as "the mount namespace
cannot be copied" on Railway, and with Simple Isolation the network policy is
disabled (0268, 0271).

### The image

`ghcr.io/demi-sh/demi`, for `linux/amd64` and `linux/arm64`:

- **Tags:** the version, such as `0.2.6017`; `canary`, the newest release;
  `beta` and `latest`, the newest Beta. An image's channel sets
  `DEMI_UPGRADE_CHANNEL`, so a server from `canary` follows Canary in
  Settings › Updates. A version's tag follows the channel whose releases
  include it, as `install.sh --version` does: a Canary's image says
  `canary` until the release becomes a Beta, and the promotion builds it
  again as a Beta.
- **Contents:** Ubuntu 26.04 with the packages the manager runs, the users
  `demi-cloud` (group 990) and `demi` (991) as `setup` creates them, with
  fixed ids since the volume's files keep them from one image to the next,
  the release's two archives and `SHA256SUMS` in `/usr/lib/demi/assets/`, and
  that release's `demi-server` as `/usr/lib/demi/demi-server`. The archives
  stay packed: the image is a local mirror of one release's assets.
  `/usr/local/bin/demi-server` links to the running release's, as `setup`
  links it on a machine, so `docker exec … demi-server status` works.
- **Entry point:** `/usr/lib/demi/demi-server container`, PID 1.
- **Defaults:** `DEMI_BACKEND_LISTEN=0.0.0.0:3271`, and the data directories of
  the layout. A platform's variables set the rest, at least
  `DEMI_BACKEND_PUBLIC_URL`.

A new workflow, `image.yml`, builds it from the release's own assets once
they are published, so an image never holds what its release does not. The
release workflow calls it for each release it publishes, and a pre-release
made the latest release starts it as a Beta.

### Starting

`demi-server container` decides which release runs, then becomes that
release's supervisor:

```text
volume /opt/demi           image                 then
nothing set up             assets of 0.2.6017    set up 0.2.6017 from the image
current 0.2.6017           assets of 0.2.6100    upgrade to 0.2.6100 from the image
current 0.2.6100           assets of 0.2.6017    run 0.2.6100 (Settings › Updates moved it)
a move interrupted         any                   finish the move, as the next command does
```

1. It writes `config/demi.env` from the container's `DEMI_*` and `AWS_*`
   variables. In a container the platform's variables are the configuration:
   the operator changes them in the platform, and the file follows at the next
   start. Nothing else writes it. It also removes what Demi's programs left in
   `/run`, which outlives a restart of the same container where a machine's
   is empty after each boot.
2. Setting up and upgrading from the image are `setup` and `upgrade` with the
   image's assets as their source: the same fetch, checks, database copies,
   journal and rollback, reading `/usr/lib/demi/assets/` as
   `fetch::Source::Directory`, which a server without internet access uses
   today.
3. It `exec`s `/opt/demi/current/bin/demi-server supervise`, so the supervisor
   is always the running release's.

A machine without systemd mounts nothing at boot, so the Cloud store there is
a directory on the volume, never the XFS loop file `setup` makes on a VPS.

### The supervisor

`demi-server supervise` does in a container what systemd does on a VPS, from
the units the release carries (`systemd/`), so a release that changes how a
service runs brings the change itself:

| The units ask | The supervisor |
| --- | --- |
| `Type=notify`, no start timeout | Gives each service a `NOTIFY_SOCKET` and waits for `READY=1` |
| `User=`, `Group=`, `SupplementaryGroups=`, `UMask=` | Starts the manager as root and the backend as `demi` |
| `RuntimeDirectory=` | Creates `/run/demi-backend`, `/run/demi-backend-manage`, `/run/demi-upgrade` and `/run/demi-cloud` |
| `Restart=on-failure`, `RestartSec=3` | Restarts a service that fails, 3 s later |
| `KillMode=process` | Signals the manager alone: Clouds it leaves run on, and the supervisor, as PID 1, reaps what they leave |
| `ExecStopPost=… --recover` | Runs the manager's recovery after it stops |
| `demi-upgrade.path`, `demi-prepare.path` | Runs `demi-server upgrade --requested` or `prepare --requested` when the backend writes the request |
| The machine's shutdown | On `SIGTERM` or `SIGINT`, stops the backend, then the manager, then exits |

`demi-server`'s `Services` (`crates/server/src/services.rs`) gains a second
implementation beside `Systemd`, which asks the supervisor through its
socket, `/run/demi-server/supervisor.sock`:

```rust
/// What `demi-server` asks of the supervisor in a container, one line of JSON
/// each way.
pub enum SupervisorRequest {
    Stop { unit: String },
    /// Answers once the service reports ready or failed.
    Start { unit: String },
    /// Reads the units of `current` again, and runs the update paths of `paths`.
    Reload { paths: Vec<String> },
    Log { unit: String },
    Screen { program: PathBuf, config: PathBuf, record: PathBuf },
    EndScreen,
    /// The manager's restart of itself, after it downloads gVisor.
    Restart { unit: String },
}
```

These types live in a crate of their own, `supervisor-protocol`, which
`demi-server` and the manager share.

`demi-server` takes `Supervisor` when the socket exists and `Systemd`
otherwise. Before the supervisor starts a service, it `exec`s
`/opt/demi/current/bin/demi-server supervise` when `current` names another
release than its own, so after an upgrade's switch the new release
supervises. The services are stopped at that point of a switch; what still
runs, the update screen, the move itself and each service's log, goes to the
next supervisor in a variable, `DEMI_SUPERVISOR_STATE`, and the Clouds a
manager left are taken over by the next manager as under systemd.

A container's processes start in the root of its cgroup hierarchy, where
cgroup v2 enables no controller for the cgroups beneath while processes live
in it. So a supervisor that may write the hierarchy, as in a privileged
container, first moves the container's processes into a cgroup of their own,
`demi-server`, as Docker's docker-in-docker image does; the manager's limits
then work beneath it.

The manager restarts itself after it downloads gVisor
(`machine_manager::handover::restart`) by `systemctl restart`, and it tells
whether systemd runs it by `INVOCATION_ID`. Both become one question, *who
restarts me*, answered by `DEMI_SERVICE_MANAGER=systemd|supervisor`, which
the units and the supervisor set; the supervisor's answer is its socket's
`Restart { unit }`.

### Each platform

The files live in `deploy/`. Each setup has a short guide of its own in
`docs/delivery/`, which the README's buttons open, and the installation guide
compares them with their monthly costs:

- **Railway** (`deploy/railway/template.json`): a template of the image, with
  a volume at `/opt/demi`, a Bucket, a domain on port 3271, a health check on
  `/api/update`, and these variables, so no value is typed:
  - `DEMI_BACKEND_PUBLIC_URL=https://${{RAILWAY_PUBLIC_DOMAIN}}`
  - `DEMI_STORAGE=s3`, `DEMI_S3_BUCKET=${{Bucket.BUCKET}}`,
    `DEMI_S3_ENDPOINT=${{Bucket.ENDPOINT}}`, `DEMI_S3_REGION=${{Bucket.REGION}}`,
    `AWS_ACCESS_KEY_ID=${{Bucket.ACCESS_KEY_ID}}`,
    `AWS_SECRET_ACCESS_KEY=${{Bucket.SECRET_ACCESS_KEY}}`
  - `PORT=3271`, the port Railway's health check reaches; Demi does not read
    it.
  - `RAILWAY_DEPLOYMENT_DRAINING_SECONDS=120`, so a redeploy gives the backend
    time to put its Clouds to sleep.

  The file is the template's serialized configuration, in the schema
  Railway's API deploys. Railway keeps a published template only in its own
  service, so a check compares the file with the published one, which
  Railway's public API returns.
- **Fly** (`deploy/fly/fly.toml`): one `shared-cpu-1x` machine with 1 GB, a
  volume at `/opt/demi`, `kill_signal = "SIGTERM"`, `kill_timeout = 300`,
  `auto_stop_machines = "off"`, and a check with Fly's longest grace period, a
  minute, past the first start. Its `[env]` sets `DEMI_MANAGED_DNS` and
  `DEMI_MANAGED_LIMITS=off`, so the master can choose gVisor: a Fly machine
  names only Fly's internal resolver, which a gVisor Cloud cannot use, and
  has cgroup v1 alone. The guide's block runs `fly launch` with it,
  `fly storage create`, and `fly secrets set` with the public URL and the
  `DEMI_S3_*` values of the bucket it made, then `fly deploy`.

  `fly storage create` also sets `AWS_ENDPOINT_URL_S3`, `AWS_REGION` and
  `BUCKET_NAME`, and the S3 client reads `AWS_*` variables over what it was
  given. Demi's own `DEMI_S3_*` settings therefore take precedence over the
  bucket, endpoint, region and addressing style `AWS_*` variables name; only
  the credentials come from `AWS_*`.
- **Docker Compose** (`deploy/compose/`): Demi and Caddy, which obtains the
  certificate for `DEMI_DOMAIN`; storage is the volume unless the operator
  sets `DEMI_STORAGE=s3`. An operator with a proxy of their own drops the Caddy
  service and publishes port 3271.

Demi reads no platform's own variable names: the templates map them to
Demi's.

### What goes where

| Data | Where | Why |
| --- | --- | --- |
| Databases | The volume | SQLite needs a local filesystem |
| Each user's Cloud, system and home | The volume | Simple Isolation runs it as a directory tree |
| Releases, gVisor, configuration, upgrade state | The volume | Settings › Updates writes them |
| Attachments and media | The platform's bucket on Railway and Fly, the volume with Compose | They grow fastest, and a bucket costs a tenth of a volume |

Demi has no move from local storage to S3, so the templates choose the bucket
from the start.

## Spike

On November 5, 2026, with Demi 0.2.6017 in an image like the one above:

- **Fly**, a trial account, `shared-cpu-2x` with 2 GB in `nrt`:
  - The machine is a VM: root with every capability, no seccomp, and
    `unshare -m` works. Its cgroups are v1, so gVisor's limits would be off.
  - Demi ran from the image with its data on a 10 GB volume: sign-up over
    `https://<app>.fly.dev`, a model's turn, and in a Cloud `sudo apt-get
    install jq`.
  - The first start took 30 s to import the Cloud base before the backend
    listened, which `fly deploy` reported as "not listening".
  - A stop sends `SIGINT` and waits 5 s by default. A trial machine is stopped
    after 5 minutes, which a card lifts, and Tigris needs a card.
- **Railway**, the Free plan, a probe image:
  - The container runs as root with Docker's default capabilities and a
    seccomp filter, without `CAP_SYS_ADMIN`: `unshare -m` fails, and proot
    from the release runs `id` as a faked root. The manager without a mount
    namespace disables gVisor only (`crates/machine-manager/src/main.rs`).
  - A redeploy sent `SIGTERM` and killed the container 9.5 s later, and the
    new one started 2 s after that, with no overlap, since the volume mounts
    on one container at a time.
  - The Free plan's volume is 434 MB and its memory 0.5 GB, too small for
    Demi; a Bucket can be created on it.
- **Sizes:** the Cloud base is 94 MB packed and 300 MB unpacked, and a Simple
  Isolation Cloud's system is a full copy of it. A release is 239 MB. On Fly,
  one Cloud with its base took 631 MB and the databases 1.8 MB. A server for
  one user therefore starts near 1.1 GB with the release before kept for a
  rollback, 1.4 GB while an update is ready ahead, and grows with what its
  users install; Railway Hobby's default 5 GB volume holds that.

### Acceptance

On November 5 and 6, 2026, with images built as above from 0.2.6017 and the
new `demi-server`:

- **Docker on arm64:** a fresh setup; a Cloud's commands; `docker stop` in
  1.3 s; a restart; an upgrade from the image to a newer release, a rollback,
  and an upgrade again, each with the supervisor's hand-over and the update
  screen answering; a failed request file reported and removed; gVisor
  downloaded, the manager restarted, and Clouds running in gVisor with
  limits in a privileged container; Simple Isolation, with gVisor's reason,
  in an unprivileged one.
- **Fly**, by the guide's commands with a Tigris bucket: setup, sign-up, an
  attachment in the bucket, a Cloud's commands, gVisor chosen in Settings ›
  Cloud and the Cloud moved to it in 27 s, and a restart listening again in
  5 s. The server used 300 MB to 340 MB with a Cloud running, and the volume
  0.9 GB to 1 GB; a `shared-cpu-1x` machine with 1 GB ran a gVisor Cloud with
  680 MB available. That machine and the volume cost about $8 a month.
- **Railway** Hobby, by the template's file deployed through Railway's API:
  the bucket's variables and the public URL filled in with nothing typed,
  setup, sign-up, an attachment in the Bucket, a Cloud's commands with
  `sudo apt-get install jq`, gVisor shown unavailable with its reason, and a
  redeploy answering again in 39 s with the Cloud's files kept. The server
  used 179 MB without a Cloud and 567 MB with one, most of it the page
  cache; the volume held 1.1 GB. That is about $2 to $6 a month of usage.

## Prior art

- **GitLab's Docker image** runs several services under runit as the
  container's init, keeps all state on volumes, and migrates on start when a
  newer image runs on older data
  ([GitLab in Docker](https://docs.gitlab.com/install/docker/)). Demi does the
  same with its own supervisor, since its upgrade already knows how to stop,
  move and start its services, and updates in place as well.
- **linuxserver.io's Plex image** updates Plex at start when `VERSION=latest`,
  so the version can be newer than the image's
  ([docker-plex](https://github.com/linuxserver/docker-plex)); Demi's in-place
  updates are that, with Demi's own checks and rollback.
- **s6-overlay** supervises several services in one container
  ([s6-overlay](https://github.com/just-containers/s6-overlay)); Demi's units
  need readiness, its upgrade's requests and a supervisor that follows the
  release, which s6 would carry as a second definition of each service.
- **Railway templates** with a bucket:
  [Openinary](https://docs.openinary.dev/guides/railway-deployment) and
  [mozilla-ai/otari](https://github.com/mozilla-ai/otari/issues/1852), which
  also keeps the template in its repository and checks it against Railway's.

## Alternatives

- **Updates only by a new image.** Settings › Updates would only say to
  redeploy, and an operator who never pulls stays behind. Rejected: Demi
  updates itself everywhere else, and the image as a seed keeps both.
- **s6-overlay or runit as PID 1.** Each service would be defined twice, for
  systemd and for the container, and the upgrade's switch would still need a
  way to drive them.
- **Demi reading `RAILWAY_PUBLIC_DOMAIN`, Fly's `AWS_ENDPOINT_URL_S3` and the
  like.** Every platform's names would become Demi's; the templates map them
  instead.
- **Render, Northflank, Koyeb, Zeabur.** Left for later decisions. Render has
  a button and a disk, which may be mounted at `/opt/demi`, so it needs a
  spike of its own before a setup.
- **Heroku, DigitalOcean App Platform, Cloud Run, Vercel.** No block volume,
  and SQLite needs one.

## Open questions

None.
