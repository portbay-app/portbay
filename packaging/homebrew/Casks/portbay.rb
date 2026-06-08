# Template for the portbay-app/homebrew-portbay tap.
# The release workflow rewrites version, sha256, and url for each tag.
cask "portbay" do
  version "0.1.0"
  sha256 "REPLACE_WITH_RELEASE_DMG_SHA256"

  url "https://github.com/portbay-app/portbay/releases/download/v#{version}/PortBay-#{version}.dmg"
  name "PortBay"

  depends_on arch: :arm64
  desc "Lightweight local development environment manager"
  homepage "https://portbay.app"

  app "PortBay.app"
  binary "#{appdir}/PortBay.app/Contents/MacOS/portbay"

  zap trash: [
    "~/Library/Application Support/com.portbay-app.portbay",
    "~/Library/Application Support/PortBay",
    "~/Library/Caches/com.portbay-app.portbay",
    "~/Library/Logs/PortBay",
    "~/Library/Preferences/com.portbay-app.portbay.plist",
    "~/Library/WebKit/com.portbay-app.portbay",
  ]
end
