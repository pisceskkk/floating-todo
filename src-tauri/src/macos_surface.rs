//! macOS native frosted glass (NSVisualEffectView) surface management.
//!
//! Unlike `tauri::Window::set_effects`, this module:
//! 1. Properly removes all `NSVisualEffectView` subviews when disabling vibrancy
//!    (preventing stale frosted borders in flat mode or multi-card floating modes).
//! 2. Insets the `NSVisualEffectView` frame by `inset` pt so the blur aligns
//!    pixel-for-pixel with `.panel` (24px radius) and `.mini-bar` (22px radius)
//!    while leaving the 12px shadow/resize gutter completely transparent.

use objc2::{msg_send, ClassType};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use tauri::{Theme, WebviewWindow};

pub fn apply_surface(
    window: &WebviewWindow,
    vibrancy: bool,
    theme: &str,
    radius: f64,
    inset: f64,
) -> Result<bool, String> {
    let target_theme = match theme {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    };
    let _ = window.set_theme(target_theme);
    let _ = window.set_shadow(false);

    let win = window.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Result<bool, String>>();
    window
        .run_on_main_thread(move || {
            let result = unsafe { apply_on_main_thread(&win, vibrancy, radius, inset) };
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;

    rx.recv()
        .map_err(|_| "macos surface update channel closed".to_string())?
}

unsafe fn apply_on_main_thread(
    window: &WebviewWindow,
    vibrancy: bool,
    radius: f64,
    inset: f64,
) -> Result<bool, String> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "apply_surface must run on macOS main thread".to_string())?;
    let raw_ns_window = window.ns_window().map_err(|error| error.to_string())?;
    if raw_ns_window.is_null() {
        return Err("ns_window pointer is null".to_string());
    }
    let ns_window: &NSWindow = &*(raw_ns_window as *const NSWindow);
    let Some(content_view) = ns_window.contentView() else {
        return Ok(false);
    };

    // Always clear every existing NSVisualEffectView so disabling vibrancy or
    // switching modes never leaves stale frosted borders behind.
    let subviews = content_view.subviews();
    for i in (0..subviews.count()).rev() {
        let subview = subviews.objectAtIndex(i);
        if subview.isKindOfClass(NSVisualEffectView::class()) {
            subview.removeFromSuperview();
        }
    }

    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));

    if !vibrancy {
        ns_window.invalidateShadow();
        return Ok(false);
    }

    let bounds = content_view.bounds();
    let pad = inset.max(0.0);
    let width = (bounds.size.width - pad * 2.0).max(0.0);
    let height = (bounds.size.height - pad * 2.0).max(0.0);
    let frame = NSRect::new(
        NSPoint::new(bounds.origin.x + pad, bounds.origin.y + pad),
        NSSize::new(width, height),
    );

    let effect_view = NSVisualEffectView::initWithFrame(mtm.alloc(), frame);
    effect_view.setMaterial(NSVisualEffectMaterial::HUDWindow);
    effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect_view.setState(NSVisualEffectState::Active);
    effect_view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    effect_view.setWantsLayer(true);
    let corner = radius.max(0.0);
    let () = msg_send![&*effect_view, setCornerRadius: corner];

    content_view.addSubview_positioned_relativeTo(&effect_view, NSWindowOrderingMode::Below, None);
    ns_window.invalidateShadow();

    Ok(true)
}
