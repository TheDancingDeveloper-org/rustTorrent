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
