//! Touchpad pinch-to-zoom on Windows.
//!
//! The viewport already zooms on a wheel event with `ctrlKey` set, which is how
//! Chromium reports a touchpad pinch to a page (`renderer.ts`). On Windows that
//! event never arrived: wry turns WebView2's `IsPinchZoomEnabled` off together
//! with the zoom hotkeys (`zoom_hotkeys_enabled`, off by default), and with it
//! off the pinch gesture is dropped before the page sees anything. Reported on
//! 7.0.0: pinching the touchpad did nothing.
//!
//! Only pinch is turned back on — not the zoom hotkeys, which would let
//! Ctrl+plus scale the whole interface. The page cancels every Ctrl+wheel
//! (`main.tsx`), so the browser's own page-scale zoom never runs; the viewport
//! zooms the document instead.

/// Turns WebView2 pinch zoom on for one webview. Logs and carries on when the
/// runtime is too old for `ICoreWebView2Settings5`: pinch is a convenience.
#[cfg(windows)]
pub fn enable(webview: &tauri::webview::PlatformWebview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings5;
    use windows_core::Interface as _;

    // SAFETY: COM calls on the live controller wry handed us, made on the
    // event-loop thread `with_webview` runs this closure on.
    let result = unsafe {
        webview
            .controller()
            .CoreWebView2()
            .and_then(|core| core.Settings())
            .and_then(|settings| settings.cast::<ICoreWebView2Settings5>())
            .and_then(|settings| settings.SetIsPinchZoomEnabled(true))
    };
    match result {
        Ok(()) => tracing::info!("zoom cubit touchpad aktif"),
        Err(e) => tracing::warn!(error = %e, "zoom cubit touchpad tidak bisa diaktifkan"),
    }
}
