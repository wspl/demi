# Deploy on Railway

Demi's Railway template runs Demi from its container image, with a 5 GB
volume for its data and a Railway Bucket for attachments and media. It fills
in every setting, so there is nothing to type.

## Before you start

You need a Railway account on the Hobby plan or above, $5 a month with $5 of
usage included. A server with a running Cloud uses about $2 to $6 of it. The
Free plan's memory and volume are too small for Demi.

## Deploy

1. Find Demi in Railway's template marketplace and click **Deploy**.
2. Wait for the deployment to finish, about a minute.
3. Open the service's domain, and create the admin account right away:
   until it exists, anyone who reaches the address can.

Then add a model in **Settings › Providers**, and update Demi from
**Settings › Updates**.

Clouds run on Simple Isolation on Railway, which doesn't allow gVisor;
Settings › Cloud says why. A redeploy waits up to two minutes for Demi to
put its Clouds to sleep.

`deploy/railway/template.json` holds the template's description, and
`deploy/railway/check.sh <code>` compares it with the published template.
