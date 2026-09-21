//! Platform integration for the HUD window.
//!
//! The Windows implementation is the real one; the stub exists so the crate
//! still type-checks on other hosts (useful for editors and for `cargo check`
//! in CI containers without the Windows SDK).

#[cfg(windows)]
mod windows_impl;
#[cfg(windows)]
pub use windows_impl::*;

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
pub use stub::*;

/// A native window handle, passed around as an integer so the signature is the
/// same on every platform.
pub type WindowHandle = isize;

/// Which desktop compositor effect to request behind the HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// Don't touch the backdrop, whatever the window already has.
    ///
    /// Only useful when something else owns the surface. The HUD asks for
    /// [`Backdrop::None`] instead: it wants no backdrop at all, and leaving the
    /// attribute alone lets Windows apply its own default to a transparent
    /// tool window.
    Inherit,
    /// Win11 "transient window" acrylic: the frosted look, best over content.
    Acrylic,
    /// Win11 Mica: tints from the desktop wallpaper, cheaper to composite.
    Mica,
    /// Explicitly no backdrop: the webview paints its own background.
    None,
}
