// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ── Wayland / WebKitGTK compatibility on Arch Linux ───────────────
    // Set BEFORE any Tauri / GTK thread is spawned.
    std::env::set_var("GDK_BACKEND", "wayland,x11");
    // Prevents blank/black window on some Wayland compositors:
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    // Fixes sluggish rendering on certain DRM/DMA-BUF setups:
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    app_lib::run();
}
