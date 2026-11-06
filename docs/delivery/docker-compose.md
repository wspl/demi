# Run with Docker Compose

Demi's Compose file runs Demi from its container image, with Caddy in front
of it to get the certificate and serve HTTPS. It runs on any machine with
Docker, such as a home server, or one of Oracle Cloud's Always Free Arm
machines, which cost nothing.

## Before you start

- Docker with Compose on the machine.
- A domain whose DNS points at the machine, with ports 80 and 443 free.

## Run

Replace `demi.example.com` with your domain:

```sh
curl -fsSLO https://raw.githubusercontent.com/demi-sh/demi/main/deploy/compose/compose.yaml
DEMI_DOMAIN=demi.example.com docker compose up -d
```

Open your domain, and create the admin account right away: until it exists,
anyone who reaches the address can.

Then add a model in **Settings › Providers**, and update Demi from
**Settings › Updates**. Pulling a newer image and starting it,
`docker compose pull && docker compose up -d`, also updates Demi.

## Options

Each changes `compose.yaml`:

- **Your own proxy:** remove the `caddy` service and publish port 3271 of
  `demi`. The proxy passes `Host` and `Origin` unchanged and allows WebSocket
  upgrades.
- **A bucket for attachments:** add `DEMI_STORAGE: s3`, the bucket's
  `DEMI_S3_BUCKET`, `DEMI_S3_ENDPOINT` and `DEMI_S3_REGION`, and its
  `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` to the `demi` service's
  `environment`.
- **gVisor:** uncomment the two lines marked `gVisor`. Without them, Clouds
  run on Simple Isolation.
