//! wlr-layer-shell integration (Wayland only).
//!
//! Promotes the main launcher window to a real layer-shell *overlay* surface via
//! `libgtk-layer-shell`, so it sits above everything with no compositor-specific
//! window rules — the correct behavior for a launcher (wofi/rofi-wayland do the same).
//!
//! Tauri 2 on Linux uses GTK3, so this uses the GTK3 `gtk-layer-shell` crate.
//! `init_layer_shell()` must run before the window is mapped; tao may have already
//! mapped it, so we `hide()` (unmap) first. See tao issue #925.

/// Apply layer-shell properties to the main window. Call once, in setup, before
/// the window is shown. No-op off Linux and off Wayland.
#[cfg(target_os = "linux")]
pub fn apply(window: &tauri::WebviewWindow) {
    use gtk::prelude::WidgetExt;
    use gtk_layer_shell::{KeyboardMode, Layer, LayerShell};

    // gtk-layer-shell only works under Wayland; skip on X11.
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }

    let gtk_win = match window.gtk_window() {
        Ok(w) => w,
        Err(_) => return,
    };

    // init_layer_shell asserts the window is not yet mapped. tao may have mapped
    // it during an early redraw even though it's configured `visible: false`, so
    // unmap it first.
    gtk_win.hide();

    gtk_win.init_layer_shell();
    gtk_win.set_layer(Layer::Overlay);
    gtk_win.set_keyboard_mode(KeyboardMode::Exclusive);
    gtk_win.set_namespace("portunus");
    // No anchors set -> the compositor centers the surface, matching `center: true`.
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_window: &tauri::WebviewWindow) {}
