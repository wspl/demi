Ubuntu 24.04 fixtures: base-files' `/usr/share/base-files/profile` and
`/etc/profile.d/01-locale-fix.sh`, and bash's `/etc/skel/.profile`.
The scenario substitutes only the system profile.d directory path, to avoid
loading this build machine's added language-toolchain profiles. The locale
helper is the installed Ubuntu `/usr/bin/locale-check`.
