# Security policy

rustTorrent is a network-facing daemon: its HTTP API, WebUI API v2 compatibility
layer and BitTorrent listener accept connections from other machines. Please do
not report security vulnerabilities in public issues, pull requests or
discussions.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting for this repository: open the
**Security** tab and choose **Report a vulnerability**, or go directly to
<https://github.com/TheDancingDeveloper-org/rustTorrent/security/advisories/new>.

Please include:

- the affected version (release tag or commit SHA) and how you run it (binary,
  container image, desktop app, or built from source);
- the component involved, such as the native HTTP API, the WebUI API v2
  compatibility layer, authentication, the BitTorrent peer or tracker code, or
  the desktop app;
- steps to reproduce, and what you observed compared with what you expected;
- the impact you believe it has.

We will acknowledge a report, keep you informed while we investigate, and credit
you in the advisory if you wish.

## Supported versions

Security fixes are made against the latest release candidate or release tag
(currently `0.1.0-rc.5`) and the `main` branch. Earlier beta releases are not
supported; upgrade to the latest tag to receive fixes.

## Deployment notes

Until the first-run setup is completed, the web UI and API accept a setup request
from anyone who can reach the port. Complete setup before exposing the port to a
network, and bind it to `127.0.0.1` when only local access is needed. See the
README for container and compose examples.
