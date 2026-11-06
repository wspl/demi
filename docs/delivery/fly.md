# Deploy on Fly.io

Demi runs on one Fly machine, with a volume for its data and a Tigris bucket
for attachments and media. A `shared-cpu-1x` machine with 1 GB and a 10 GB
volume costs about $8 a month.

## Before you start

- Install `flyctl` and sign in with `fly auth login`.
- Add a card to the Fly account: Tigris and a machine that keeps running need
  one.

## Deploy

Run this block. It names the app `demi-` and six random characters; set `APP`
to another free name to choose your own:

```sh
APP=demi-$(openssl rand -hex 3)
curl -fsSLO https://raw.githubusercontent.com/demi-sh/demi/main/deploy/fly/fly.toml
fly launch --copy-config --no-deploy --name "$APP"
fly storage create --name "$APP-files" --yes
fly secrets set --stage --app "$APP" \
  DEMI_BACKEND_PUBLIC_URL="https://$APP.fly.dev" \
  DEMI_STORAGE=s3 DEMI_S3_BUCKET="$APP-files" \
  DEMI_S3_ENDPOINT=https://fly.storage.tigris.dev DEMI_S3_REGION=auto
fly deploy --app "$APP"
```

The first start takes about 30 seconds. Then open `https://<app>.fly.dev`,
and create the admin account right away: until it exists, anyone who reaches
the address can.

Then add a model in **Settings › Providers**, and update Demi from
**Settings › Updates**.

## Afterwards

- **gVisor:** a Fly machine is a VM, so you can choose gVisor in
  **Settings › Cloud**.
- **More memory:** `fly scale memory 2048 --app <app>`, for a server with
  more users.
