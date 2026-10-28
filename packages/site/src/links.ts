// Where the page's links lead, and the install command with what it needs.
export const REPOSITORY = 'https://github.com/wspl/demi'
export const DOCS = `${REPOSITORY}/blob/main/docs/delivery/installation.md`
export const LICENSE = `${REPOSITORY}#license`
export const INSTALL_COMMAND = 'curl -fsSL https://demi.sh/install.sh | sudo bash'
// What the server needs, said once for the hero, the install screen and
// llms.txt.
export const REQUIREMENTS = 'Requires a Linux server and a domain name.'
// Which Linux distributions the server can run: the names, as the
// installation guide's table lists them, and the table's address.
// A name never breaks across lines: each joins its words with a no-break
// space.
export const DISTRIBUTIONS = `Supports ${[
  'Ubuntu 22.04+',
  'Debian 12+',
  'Fedora',
  'RHEL 10+',
  'Rocky Linux',
  'AlmaLinux',
].map((name) => name.replaceAll(' ', '\u00a0')).join(', ')} and Arch\u00a0Linux.`
export const DISTRIBUTIONS_URL = `${DOCS}#distributions`
