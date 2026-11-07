cask "mcp-gateway-desk" do
  version "0.1.0"
  sha256 "d0ba58665fcdf51ea8589a58b1ff737bd0a8e9113873ed9b1161b8bee2915195"

  url "https://github.com/winter-wub/mcp-gateway-desk/releases/download/v#{version}/MCP-Gateway-Desk-#{version}.dmg"
  name "MCP Gateway Desk"
  desc "Manage MCP servers and connect AI apps, with direct access to local folders"
  homepage "https://github.com/winter-wub/mcp-gateway-desk"

  depends_on macos: :big_sur

  app "MCP Gateway Desk.app"
  binary "#{appdir}/MCP Gateway Desk.app/Contents/MacOS/mcp-gateway-desk"

  # The app is signed ad hoc, not notarized: clear the download flag so it opens normally.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/MCP Gateway Desk.app"]
  end

  uninstall quit: "io.github.winter-wub.mcp-gateway-desk"

  zap trash: [
    "~/.mcp-gateway-desk",
    "~/Library/LaunchAgents/com.craftanything.mcp-gateway-desk.plist",
  ]
end
