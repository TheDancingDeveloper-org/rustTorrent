# rtbit GUI Application

This is a thin tauri wrapper for the web ui of rtbit.

Platforms: **Linux and Windows only.** macOS builds are out of scope.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Dependencies

* Tauri CLI.

```
cargo install tauri-cli
```

* Nodejs and NPM

## How to build GUI

* From the rustTorrent repo root, go to `crates/librtbit/webui`
  
  ```
  npm install
  npm run build
  ```
  
* Go to `rtbit/desktop`
  
  ```
  npm install
  cargo tauri dev
  ```

## OS integration

* **Opening torrents.** Opening a `.torrent` file or a `magnet:` link, from a file manager or a browser, adds it to the running app. A second launch hands its arguments to the first and exits, so there is one engine and one port.
* **Tray.** Show, Add `.torrent` file, Pause/Resume All, Open in Browser, Start at login, Check for Updates and Quit. Closing the window hides it to the tray. Quitting with transfers active asks first.
* **Menu bar.** File, Torrent and View, using the same actions as the tray.
* **Start at login.** A toggle in the tray menu. The autostart entry starts the app hidden (`--autostart`).
* **Engine failure.** If the engine cannot start, a native error dialog shows the cause, the config path and the likely port conflict.

### Linux desktop entry

The bundled `.desktop` file comes from `linux/rtbit-desktop.desktop.hbs`. Its `Exec` line ends in `%U` so that file-manager launches pass the file and `MimeType` lists `application/x-bittorrent` and `x-scheme-handler/magnet`.

### Release signing (operator setup)

These are not in the repository, and the updater stays inert until they are set:

1. **Updater key.** Generate a Tauri updater keypair and put the public key in `plugins.updater.pubkey` in `tauri.conf.json`. Keep the private key out of Git, in the CI secret store, and set `TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) when building. Then set `bundle.createUpdaterArtifacts` to `true` so signed artifacts and `latest.json` are produced.
2. **Endpoint.** `plugins.updater.endpoints` points at the GitHub release `latest.json`. That location is a placeholder until a desktop release job publishes it.
3. **Windows code signing.** Needs a code-signing certificate. Set `bundle.windows` (`certificateThumbprint`, `digestAlgorithm`, `timestampUrl`) from the CI secret store.

With an empty `pubkey`, "Check for Updates" can still find a release, but it will refuse to install it.

## Manual verification

Nothing in the CI gate runs this crate (`ci.yml` excludes `rtbit-desktop`), so these runs are the only check of the OS integration. Do them on an installed package, not `cargo tauri dev`, except where a step says so. Build first with `cd desktop && npm install`, then `cd src-tauri && cargo tauri build`.

### Linux (deb)

1. Install: `sudo dpkg -i target/release/bundle/deb/rtbit-desktop_*.deb`.
2. Desktop entry: `grep -E '^(Exec|MimeType)' /usr/share/applications/rtbit-desktop.desktop`. Expect `Exec=... %U` and a `MimeType` that lists `application/x-bittorrent` and `x-scheme-handler/magnet`.
3. Torrent file: in the file manager, open a `.torrent` with rtbit-desktop. The torrent appears in the web UI list.
4. Magnet: `xdg-open 'magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862'`. The torrent is added.
5. Single instance: with the app running, run `rtbit-desktop /path/to/file.torrent`. The existing window is raised, the torrent is added, and `ss -ltnp | grep 3030` shows one listener from the first process.
6. Tray (use the menu, not a click; left-click does not fire on Linux AppIndicator):
   - Show brings the window back.
   - Add `.torrent` file opens a picker and adds the chosen file.
   - Pause All / Resume All flips its label as torrents change state.
   - Open in Browser opens the web UI.
   - Start at login creates `~/.config/autostart/rtbit-desktop.desktop` when checked, and removes it when unchecked.
   - Check for Updates shows a dialog. With the empty pubkey it should report a failure, not install anything.
   - Quit with an active torrent asks for confirmation. Cancel keeps the app running.
7. Close to tray: close the window. The process keeps running and the tray Show restores the window.
8. Autostart: enable Start at login, log out and back in. The window does not appear, and the tray icon does.
9. Engine failure: in a terminal, run `python3 -m http.server 3030 --bind 127.0.0.1`, then launch rtbit-desktop. A native error dialog names the cause and the config path. The process exits with code 1. Stop the server afterwards.

### Windows (nsis or msi)

1. Build with `cargo tauri build --bundles nsis` (or `msi`) and install the artifact.
2. Registry: `reg query "HKCU\Software\Classes\.torrent"` and `reg query "HKCU\Software\Classes\magnet"` (or the machine-wide equivalents). Both should point to rtbit-desktop.
3. Repeat steps 3 to 5 from Linux: double-click a `.torrent`, click a `magnet:` link in a browser, and run a second instance with a torrent path.
4. Repeat tray steps 6 to 9 from Linux. For autostart, the entry is under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

### Pass criteria

Each numbered step passes if its stated result appears. Note the build, the OS version, and any step that fails, and file the failures against WI-1733.
