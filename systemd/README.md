# Running rtbit under systemd

These files run `rtbit` as a **per-user** systemd service (`systemctl --user`),
socket-activated on `127.0.0.1:3030`. They do not install a system-wide service.

| File | Installed to |
|------|--------------|
| `rtbit.socket` | `~/.config/systemd/user/rtbit.socket` |
| `rtbit.service` | `~/.config/systemd/user/rtbit.service` |
| `rtbit.conf` | `~/.config/rtbit/rtbit.conf` (optional; every line is commented out) |

## 1. Install the binary

The unit runs the binary at `/usr/local/bin/rtbit`. Build it and copy it there:

```sh
cargo build --release -p rtbit
sudo install -m 0755 target/release/rtbit /usr/local/bin/rtbit
```

If you install elsewhere, change `ExecStart=` in `rtbit.service` to match. systemd
requires an absolute path there.

## 2. Install the unit files

```sh
mkdir -p ~/.config/systemd/user ~/.config/rtbit
install -m 0644 systemd/rtbit.socket systemd/rtbit.service ~/.config/systemd/user/
install -m 0644 systemd/rtbit.conf ~/.config/rtbit/rtbit.conf   # optional
systemctl --user daemon-reload
systemctl --user enable --now rtbit.socket
```

Enable the socket, not the service. The service starts on the first connection to
`127.0.0.1:3030`.

## 3. Check it

```sh
curl -fsS http://127.0.0.1:3030/healthz   # prints "ok"
systemctl --user status rtbit.service
journalctl --user -u rtbit.service -e
```

`/healthz` answers without credentials, so it works before first-run setup.

## Notes

- **Listen address.** The socket owns the listen address. `RTBIT_HTTP_API_LISTEN_ADDR`
  in `rtbit.conf` has no effect under socket activation; change `ListenStream=` in
  `rtbit.socket` instead.
- **Downloads.** The service serves `%h/Downloads` (your home's `Downloads` folder).
  Change the `ExecStart=` argument to use another folder.
- **Sandboxing.** The unit sets `NoNewPrivileges`, `RestrictSUIDSGID` and
  `LockPersonality`. Filesystem sandboxing is not applied; the service runs with your
  user's file access.
- **Survive logout.** User services stop at logout unless lingering is enabled:
  `loginctl enable-linger "$USER"`.
