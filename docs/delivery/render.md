# Deploy on Render

Demi runs on one Render web service, with a 5 GB disk for its data and its
attachments. The Starter instance with 512 MB and the disk costs about $8.25 a
month.

## Before you start

You need a Render account with a card on it: a service with a disk is paid.

## Deploy

1. Click **Deploy on Render** in the [README](../../README.md#quick-start).
2. Click **Deploy Blueprint**, and wait for the deployment to finish, about
   two minutes.
3. Open the service's address, and create the admin account right away: until
   it exists, anyone who reaches the address can.

Then add a model in **Settings › Providers**, and update Demi from
**Settings › Updates**.

## Afterwards

- **gVisor:** a Render service can't run it, so Clouds run on Simple Isolation.
- **More memory:** choose a larger instance in the service's **Settings**, for
  a server with more users or a Cloud that does heavy work.
- **Restarts:** a service with a disk stops for a few seconds on each deploy
  and restart.
