mod catalog;
mod clients;
mod gateway;
mod icon;
mod oauth;
mod state;
mod tray;
mod ui;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `mcp-gateway-desk gateway run` — what AI apps start (like `docker mcp gateway run`).
    if args.get(1).map(|s| s.as_str()) == Some("gateway") {
        gateway::run_stdio();
        return;
    }
    // `mcp-gateway-desk --write-icon <file.png> [size]` — used when packaging the .app.
    if args.get(1).map(|s| s.as_str()) == Some("--write-icon") {
        let path = args.get(2).cloned().unwrap_or_else(|| "icon.png".into());
        let size = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1024);
        if let Err(e) = icon::render(size).save(&path) {
            eprintln!("couldn't write {}: {}", path, e);
            std::process::exit(1);
        }
        return;
    }
    // `mcp-gateway-desk tray` — the menu-bar icon that keeps running without the window.
    if args.get(1).map(|s| s.as_str()) == Some("tray") {
        tray::run();
        return;
    }
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MCP Gateway Desk")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "MCP Gateway Desk",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, screenshot)))),
    );
}
