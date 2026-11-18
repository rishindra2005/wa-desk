//! System clipboard image write.
//!
//! GNOME's Wayland compositor (mutter) does not implement the wlroots/ext
//! data-control protocols that `arboard` needs for background clipboard
//! access, and X11-only crates depend on XWayland. GTK is already linked by
//! Tauri's webview and talks to whichever display server is active, so we
//! put the image on the clipboard with `gtk::Clipboard` on the main thread.

use tauri::{AppHandle, Wry};
use wa_core::{AppError, Result};

use crate::core::ClipboardImage;

/// Check that an image's size fits GDK's `i32` geometry and its buffer.
pub fn validate(image: &ClipboardImage) -> Result<(i32, i32)> {
    let w = i32::try_from(image.width).map_err(|_| AppError::invalid("The image is too wide"))?;
    let h = i32::try_from(image.height).map_err(|_| AppError::invalid("The image is too tall"))?;
    let expected = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|p| p.checked_mul(4));
    if w == 0 || h == 0 || expected != Some(image.rgba.len()) {
        return Err(AppError::invalid("The image data is not valid"));
    }
    Ok((w, h))
}

/// Put `image` on the clipboard as image data (not a path).
#[cfg(target_os = "linux")]
pub fn copy_image(app: &AppHandle<Wry>, image: ClipboardImage) -> Result<()> {
    let (w, h) = validate(&image)?;
    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
    app.run_on_main_thread(move || {
        use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
        let bytes = gtk::glib::Bytes::from_owned(image.rgba);
        let pixbuf = Pixbuf::from_bytes(&bytes, Colorspace::Rgb, true, 8, w, h, w * 4);
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_image(&pixbuf);
        // Hand the data to the clipboard manager so it survives the window closing.
        clipboard.store();
        let _ = tx.send(Ok(()));
    })
    .map_err(|e| AppError::internal("Could not reach the UI thread").with_detail(e.to_string()))?;
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| AppError::timeout("The clipboard did not respond"))?
}

/// Clipboard images are only implemented for Linux (the only shipped target).
#[cfg(not(target_os = "linux"))]
pub fn copy_image(_app: &AppHandle<Wry>, image: ClipboardImage) -> Result<()> {
    validate(&image)?;
    Err(AppError::external(
        "Copying images is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, len: usize) -> ClipboardImage {
        ClipboardImage {
            width: w,
            height: h,
            rgba: vec![0; len],
        }
    }

    #[test]
    fn validates_geometry() {
        assert_eq!(validate(&img(2, 3, 24)).unwrap(), (2, 3));
        assert!(validate(&img(0, 3, 0)).is_err());
        assert!(validate(&img(2, 3, 23)).is_err());
        assert!(validate(&img(u32::MAX, 1, 4)).is_err());
    }
}
