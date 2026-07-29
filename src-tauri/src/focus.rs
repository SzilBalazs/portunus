//! Reliable show + keyboard focus for the launcher window.
//!
//! `WebviewWindow::set_focus()` is unreliable right after `show()` on Linux:
//! tao's `set_focus` early-returns unless the GTK window already reports
//! visible, and both `show()` and `set_focus()` only *queue* a request for the
//! GTK main thread. Called back-to-back from a worker thread (the IPC socket
//! handler) the Visible request hasn't been processed yet, so the Focus request
//! is silently dropped and the window maps unfocused. Compositors that focus on
//! map anyway (KDE, wlroots + our layer-shell exclusive keyboard mode) hide the
//! bug; GNOME/mutter does not - it applies focus-stealing prevention and the
//! launcher comes up unable to type.
//!
//! Fix: ask GTK directly, on the main thread, via `gtk_window().present()` -
//! which maps, raises *and* focuses in one go, so it works regardless of how it
//! interleaves with the queued Visible request. Remove once tao's `set_focus`
//! guard is fixed upstream (tauri-apps/tauri#6310).

/// Show the window and give it keyboard focus.
pub fn show_and_focus(window: &tauri::WebviewWindow) {
    let _ = window.show();
    focus(window);
}

/// Give an already-visible window keyboard focus.
#[cfg(target_os = "linux")]
pub fn focus(window: &tauri::WebviewWindow) {
    let win = window.clone();
    let _ = window.run_on_main_thread(move || {
        use gtk::prelude::GtkWindowExt;
        match win.gtk_window() {
            Ok(gtk_win) => gtk_win.present(),
            // No GTK window (shouldn't happen) - fall back to the tao path.
            Err(_) => {
                let _ = win.set_focus();
            }
        }
    });
}

#[cfg(not(target_os = "linux"))]
pub fn focus(window: &tauri::WebviewWindow) {
    let _ = window.set_focus();
}
