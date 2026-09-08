import { Hono } from 'hono'
import { releaseDigest as digest, releaseTarget as target, runnerReleaseSchema as releaseSchema, type RunnerRelease } from '@demicodes/runner-protocol/release'
export type { RunnerRelease } from '@demicodes/runner-protocol/release'
import { createHash } from 'node:crypto'
import { join } from 'node:path'
export interface RunnerInstallationOptions { directory: string; backendUrl?: string }

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`

export function shellInstaller(backend: string, release: RunnerRelease): string {
  const url = new URL(backend)
  if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw new Error('invalid installation backend URL')
  const registration = createHash('sha256').update(url.toString()).digest('hex')
  const cases = Object.entries(release.targets).map(([name, hashes]) => `  ${name}) runner_hash=${quote(hashes!.runner)}; client_hash=${quote(hashes!.client)} ;;`).join('\n')
  return `#!/bin/sh
set -eu
backend=${quote(url.toString())}
release=${quote(release.release)}
registration=${quote(registration)}
base=${quote(url.origin)}
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) target=macos-arm64 ;;
  Darwin:x86_64) target=macos-x64 ;;
  Linux:aarch64|Linux:arm64) target=linux-arm64 ;;
  Linux:x86_64) target=linux-x64 ;;
  *) echo 'Unsupported runner platform' >&2; exit 1 ;;
esac
case "$target" in
${cases}
  *) echo 'This backend has no runner release for this platform' >&2; exit 1 ;;
esac
instance=\${DEMI_INSTALLATION_ID:-$registration}
case "$instance" in ''|*[!a-zA-Z0-9_-]*) echo 'Invalid installation ID' >&2; exit 1 ;; esac
state="$HOME/.demi/instances/$instance"
bin="$state/releases/$release"
umask 077
mkdir -p "$state/releases"
if [ -f "$state/backend-url" ]; then
  IFS= read -r existing_backend < "$state/backend-url"
  [ "$existing_backend" = "$backend" ] || { echo 'Installation belongs to another backend' >&2; exit 1; }
fi
mkdir "$state/install.lock" 2>/dev/null || { echo 'Another installer is active for this installation' >&2; exit 1; }
stage=$(mktemp -d "$state/releases/.download-XXXXXX")
trap 'rm -r "$stage"; rmdir "$state/install.lock"' EXIT
trap 'exit 1' HUP INT TERM
verify() {
  if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$1"); else actual=$(shasum -a 256 "$1"); fi
  actual=\${actual%% *}
  [ "$actual" = "$2" ] || { echo 'Runner download checksum mismatch' >&2; exit 1; }
}
curl -fSL "$base/runner-artifacts/$release/$target/demi-runner" -o "$stage/demi-runner"
curl -fSL "$base/runner-artifacts/$release/$target/demi" -o "$stage/demi"
verify "$stage/demi-runner" "$runner_hash"
verify "$stage/demi" "$client_hash"
chmod 755 "$stage/demi-runner" "$stage/demi"
if [ -d "$bin" ]; then
  verify "$bin/demi-runner" "$runner_hash"
  verify "$bin/demi" "$client_hash"
else
  mv "$stage" "$bin"; mkdir "$stage"
fi
if [ -x "$state/run" ]; then
  current=$("$state/run" status 2>/dev/null || true)
  if [ "$current" = "running $release" ]; then echo "Runner already running: $state"; exit 0; fi
  if [ -n "$current" ]; then echo 'Waiting for existing jobs before upgrading this runner…'; "$state/run" drain; fi
fi
# Pass values as quoted arguments; never interpolate a backend into executable shell code.
cat > "$state/run.next" <<'LAUNCHER'
#!/bin/sh
set -eu
state=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IFS= read -r backend < "$state/backend-url"
IFS= read -r release < "$state/release-id"
export DEMI_HOME="$state" DEMI_RELEASE_ID="$release"
exec "$state/releases/$release/demi-runner" "\${1:-run}" --backend "$backend"
LAUNCHER
printf '%s\\n' "$backend" > "$state/backend-url"
printf '%s\\n' "$release" > "$state/release-id"
chmod 755 "$state/run.next"
mv "$state/run.next" "$state/run"
nohup "$state/run" > "$state/runner.log" 2>&1 < /dev/null &
pid=$!
tries=0
until "$state/run" status >/dev/null 2>&1; do
  tries=$((tries + 1))
  if ! kill -0 "$pid" 2>/dev/null || [ "$tries" -ge 100 ]; then cat "$state/runner.log" >&2; exit 1; fi
  sleep 0.1
done
printf 'Runner installed for %s\\nState and pairing log: %s\\n' "$backend" "$state/runner.log"
cat "$state/runner.log"
`
}

/** Public downloads contain no credential; device access still requires pairing. */
export function runnerInstallRoutes(options?: RunnerInstallationOptions): Hono {
  const app = new Hono()
  const manifest = async () => {
    if (!options) return null
    return releaseSchema.parse(await Bun.file(join(options.directory, 'manifest.json')).json())
  }
  app.get('/install.sh', async c => {
    const release = await manifest()
    if (!release) return c.text('Runner releases are not configured on this backend.\n', 503)
    return c.body(shellInstaller(options!.backendUrl ?? new URL(c.req.url).origin, release), 200, { 'content-type': 'text/x-shellscript; charset=utf-8', 'cache-control': 'no-store' })
  })
  app.get('/install.ps1', c => c.text('A Windows runner release is not available yet.\n', 503))
  app.get('/runner-artifacts/:release/:target/:file', async c => {
    if (!options) return c.notFound()
    const wanted = digest.safeParse(c.req.param('release')), platform = target.safeParse(c.req.param('target'))
    const name = c.req.param('file')
    if (!wanted.success || !platform.success || !['demi', 'demi-runner'].includes(name)) return c.notFound()
    const releaseFile = Bun.file(join(options.directory, wanted.data, 'manifest.json'))
    if (!await releaseFile.exists()) return c.notFound()
    const current = releaseSchema.parse(await releaseFile.json())
    if (current?.release !== wanted.data || !current.targets[platform.data]) return c.notFound()
    const file = Bun.file(join(options.directory, wanted.data, platform.data, name))
    if (!await file.exists()) return c.notFound()
    return new Response(file, { headers: { 'content-type': 'application/octet-stream', 'cache-control': 'public, max-age=31536000, immutable' } })
  })
  return app
}
