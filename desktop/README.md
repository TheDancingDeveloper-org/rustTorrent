# rtbit GUI Application

This is a thin tauri wrapper for the web ui of rtbit.

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
