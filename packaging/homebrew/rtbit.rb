# Homebrew formula for rtbit on Linux. Installs the signed release binary,
# not a source build. This file is a template: packaging/scripts/render-homebrew.sh
# fills the placeholder values from the verified SHA256SUMS of the release, and
# the rendered formula is what gets copied into the tap.
class Rtbit < Formula
  desc "BitTorrent client written in Rust, with a web UI and HTTP API"
  homepage "https://github.com/TheDancingDeveloper-org/rustTorrent"
  version "REPLACE_WITH_VERSION"
  license "Apache-2.0"

  on_linux do
    on_intel do
      url "https://github.com/TheDancingDeveloper-org/rustTorrent/releases/download/REPLACE_WITH_TAG/rtbit-REPLACE_WITH_TAG-linux-x86_64"
      sha256 "REPLACE_WITH_SHA256_LINUX_BINARY"
    end
  end

  def install
    bin.install Dir["rtbit-*-linux-x86_64"].first => "rtbit"
  end

  test do
    assert_match "rtbit", shell_output("#{bin}/rtbit --version")
  end
end
