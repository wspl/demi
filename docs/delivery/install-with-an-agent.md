# Install Demi with an agent

You are an AI agent, and the user asked you to install Demi, a self-hosted AI
agent on the web, on their Linux server. Follow these steps. The
[installation guide](https://github.com/wspl/demi/blob/main/docs/delivery/installation.md)
has every detail they leave out.

Demi is installed when `https://<domain>` opens Demi's setup page and the
installer's last step, its check from outside, has passed.

## 1. Ask the user

Ask only what you cannot find out yourself:

- **The server.** Whether you run on it already or reach it over SSH, and as
  which user. That user needs root or `sudo`.
- **The domain.** Demi is reached at `https://<domain>`, never at an IP
  address.
- **HTTPS.** Demi serves plain HTTP behind something that serves the domain
  over HTTPS. Ask which one the user has or wants, and offer these:
  - Cloudflare's proxy with the SSL/TLS mode Flexible. It is the simplest:
    nothing to install on the server.
  - Caddy on the server, which gets its own certificate.
  - A reverse proxy the server runs already, such as nginx, or a tunnel, such
    as Cloudflare Tunnel.
- **The channel.** Beta, the releases Demi publishes, unless the user wants
  Canary, which gets every build first.

## 2. Check the server

On the server, check each of these, and tell the user what fails:

- Linux with systemd, on a
  [supported distribution](https://github.com/wspl/demi/blob/main/docs/delivery/installation.md#distributions)
  (`/etc/os-release`).
- No Demi yet: `/opt/demi/current` does not exist. If it does, Demi is
  installed, and `sudo demi-server upgrade` moves it to a newer release.
- The domain resolves to the server, or to Cloudflare when its proxy is on.

## 3. Set up HTTPS

Set up the user's choice before you install, and change nothing else on the
server without asking. Whatever serves HTTPS must pass `Host` and `Origin`
unchanged, allow WebSocket upgrades and forward to the address Demi listens
on:

- Cloudflare's proxy: Demi listens on `0.0.0.0:80`.
- A proxy or a tunnel on the server: Demi listens on `127.0.0.1:3271`.

[HTTPS](https://github.com/wspl/demi/blob/main/docs/delivery/installation.md#https)
says how to set up each one.

## 4. Install

Run the installer with the domain and the address:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --domain <domain> --listen <address>
```

For Canary, add `--canary`. Without a terminal, the installer asks nothing:
when a parameter is missing, it prints its guide and the command to run
again, and exits with 0. Read the guide, and run that command.

If a step fails, the installer says what failed and what to do. Fix it, and
run the same command again: it continues where it stopped. If it says that
Beta has no release yet, ask the user whether to install Canary instead.

Never pass a secret on the command line, which other users of the machine can
read. The installer reads S3 credentials from its environment.

## 5. Hand over

Tell the user:

- To open `https://<domain>` right away and create the admin account. Until
  that account exists, anyone who reaches the domain can create it, so do not
  create it yourself.
- The channel you installed, and the release that `sudo demi-server status`
  prints.
- What you set up for HTTPS, and anything else you changed on the server.
