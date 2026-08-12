/// macOS-only Accessibility permission module.
///
/// On macOS, synthetic keyboard injection (enigo) and system-wide hotkey listening
/// both require the user to grant **Accessibility** permission in System Settings.
/// This module provides:
///
/// 1. A silent check `is_trusted()` — returns whether the app already has permission.
/// 2. A one‑shot prompt `request_once()` — fires the system Accessibility dialog
///    the first time recording starts.  Guarded by `std::sync::Once` so it only
///    fires once per process lifetime.
/// 3. A fallback `open_settings()` — opens the exact System Settings pane so the
///    user can grant permission manually.  Also guarded by `Once`.
///
/// On Windows and Linux all functions are no‑ops (`is_trusted()` returns `true`).
///
/// References:
///   * Apple Documentation: AXIsProcessTrusted
///   * CGRequestPostEventAccess — triggers the TCC prompt (macOS 10.15+)
///   * LanMouse (open source) implements a very similar pattern.
///
#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> i8;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGRequestPostEventAccess() -> i8;
}

/// Returns `true` on macOS if the app has Accessibility permission, otherwise `false`.
/// On non‑macOS platforms this always returns `true` (no permission required).
pub fn is_trusted() -> bool {
    #[cfg(target_os = "macos")]
    unsafe { AXIsProcessTrusted() != 0 };
    #[cfg(not(target_os = "macos"))]
    true
}

/// Fires the system Accessibility permission prompt once.
/// On macOS: if `is_trusted()` is false, calls `CGRequestPostEventAccess()` which
/// pops the TCC alert.  The `Once` guard ensures it only runs the first time.
/// On non‑macOS: no‑op.
pub fn request_once() {
    static FIRED: std::sync::Once = std::sync::Once::new();
    FIRED.call_once(|| {
        #[cfg(target_os = "macos")]
        {
            if !is_trusted() {
                // Trigger the TCC prompt.  The return value is ignored — the
                // system dialog will appear regardless.
                let _ = unsafe { CGRequestPostEventAccess() };
            }
        }
        #[cfg(not(target_os = "macos"))]
        {}
    });
}

/// Opens the macOS System Settings pane for Accessibility.
/// Guarded by `Once` so it opens only once per process lifetime.
/// On non‑macOS: no‑op.
pub fn open_settings() {
    static FIRED: std::sync::Once = std::sync::Once::new();
    FIRED.call_once(|| {
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                .spawn();
        }
        #[cfg(not(target_os = "macos"))]
        {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_trusted_always_true_on_non_mac() {
        assert!(is_trusted());
    }
}