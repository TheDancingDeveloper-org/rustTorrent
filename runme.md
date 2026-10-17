To start the app/stack, you have a few options:

  Quick dev server (simulates traffic, no real torrents needed):
  make testserver
  This runs at http://localhost:3030 with a simulated session.

  WebUI in dev mode (hot reload):
  make webui-dev
  Runs at http://localhost:3031, proxies API calls to :3030 (so run testserver first).

  Run the actual binary:
  export PATH="$HOME/.cargo/bin:$PATH"
  cargo build --release
  ./target/release/rtbit --help

  With WebUI built in:
  cargo build --release --features webui   # requires npm installed
  ./target/release/rtbit server start /path/to/downloads
  Then open http://localhost:3030.

  Mock UI (no backend at all):
  cd crates/librtbit/webui && npm run dev:mock
  Opens http://localhost:3032/mock.html with 1000 fake torrents.

Release and CI process:
  See docs/ReleaseProcess.md.
