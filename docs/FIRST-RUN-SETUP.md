# First-run setup

Until an admin account exists, rtbit's HTTP API answers only `/auth/status`,
`/auth/setup` and the web UI. Everything else returns `403 setup required`.
Setup needs a one-time **setup token**, so a machine on the network cannot
claim the admin account by reaching the port first.

## Where the token is

The token is printed in three places, and all three are removed or invalidated
once setup succeeds:

- **Desktop app:** injected into the app's own window, so the setup page fills
  the field in. Nothing needs to be copied.
- **Server log:** printed to stderr at startup, for example `docker logs <container>`
  or `journalctl -u rtbit`. It is written to stderr rather than the INFO log, so
  log shippers are less likely to keep a copy.
- **Token file:** `setup-token`, next to `credentials.json`, owner-only (0600).
  Use it on headless installs where nobody reads the log. It is deleted when
  setup succeeds. On Linux the server keeps it in `~/.local/share/com.rtbit.session/`
  (the same folder as `credentials.json`; `XDG_DATA_HOME` overrides the parent).
  The desktop app keeps it in its own config folder.

## Finishing setup

Open the web UI, paste the token, and choose a username and password. Or call
the API directly:

```sh
curl -X POST http://HOST:3030/auth/setup \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"...","setup_token":"<token>"}'
```

## Lost the password

Stop the server, then reset the stored credentials from the command line. The
new password is read from stdin, so it never appears in the process list:

```sh
printf '%s' 'new-password' | rtbit auth reset-password -u admin
```

This works even when `credentials.json` is corrupt. Restart rtbit afterwards so
a running server picks up the change.

## Lockouts

After five failed logins from one client address (or one IPv6 `/64`), that
client gets `429 Too Many Requests` with `Retry-After`. The delay doubles up to
one hour. The same limit applies to `/auth/login`, `/auth/change_credentials`,
the qBittorrent-compatible login, and failed HTTP Basic auth.
