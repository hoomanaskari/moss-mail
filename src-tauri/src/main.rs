// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `Moss mcp <url>` is the stdio bridge an agent like Claude Desktop starts; else the app.
    moss_lib::mcp::bridge_or(moss_lib::run)
}
