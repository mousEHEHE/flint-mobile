use std::path::{Path, PathBuf};

use gpui::App;

use crate::toast;

/// Downloads wait here to be handed over, each in its own folder so the file keeps its name.
pub fn scratch_dir() -> PathBuf {
    gpui_mobile::packages::path_provider::temporary_directory()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("flint-files")
}

/// A file stays behind when the app is killed before its sheet closes.
pub fn clear_scratch() {
    let dir = scratch_dir();
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(?e, "could not clear the downloads waiting to be shared");
        }
    }
}

/// gpui-mobile has no file share on Android yet, and its save dialog answers at once as cancelled.
pub fn can_hand_over() -> bool {
    !cfg!(target_os = "android")
}

/// Removes the folder of `path` once the phone has the file.
pub fn hand_over(path: PathBuf, cx: &mut App) {
    #[cfg(target_os = "ios")]
    {
        let _ = cx;
        if let Err(e) = ios::share(&path) {
            tracing::error!(error = e, "could not show the share sheet");
            remove_folder_of(&path);
            toast::show("The file could not be shared", toast::Kind::Problem, cx);
        }
    }
    #[cfg(not(target_os = "ios"))]
    save_as(path, cx);
}

fn remove_folder_of(path: &Path) {
    if let Some(folder) = path.parent() {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// Only the preview gets here: callers check `can_hand_over` first, which rules out Android.
#[cfg(not(target_os = "ios"))]
fn save_as(path: PathBuf, cx: &mut App) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|folder| folder.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let chosen = cx.prompt_for_new_path(&folder, Some(&name));
    cx.spawn(async move |cx| {
        let outcome = match chosen.await {
            Ok(Ok(Some(target))) => match std::fs::copy(&path, &target) {
                Ok(_) => Some(("Saved", toast::Kind::Done)),
                Err(e) => {
                    tracing::warn!(?e, "could not save the download");
                    Some(("The file could not be saved", toast::Kind::Problem))
                }
            },
            Ok(Ok(None)) => None,
            _ => Some(("The save dialog did not open", toast::Kind::Problem)),
        };
        remove_folder_of(&path);
        if let Some((text, kind)) = outcome {
            cx.update(|cx| toast::show(text, kind, cx));
        }
    })
    .detach();
}

#[cfg(target_os = "ios")]
mod ios {
    use std::ffi::CString;
    use std::path::{Path, PathBuf};

    use block2::RcBlock;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send, MainThreadMarker};

    pub fn share(path: &Path) -> Result<(), &'static str> {
        if MainThreadMarker::new().is_none() {
            return Err("the share sheet opens on the main thread only");
        }
        let text = CString::new(path.to_str().ok_or("the path is not UTF-8")?)
            .map_err(|_| "the path has a NUL in it")?;
        // SAFETY: Foundation and UIKit calls with the argument types they take, on the main thread.
        unsafe {
            let ns_path: Option<Retained<AnyObject>> =
                msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()];
            let ns_path = ns_path.ok_or("no string for the path")?;
            let url: Option<Retained<AnyObject>> =
                msg_send![class!(NSURL), fileURLWithPath: &*ns_path];
            let url = url.ok_or("no URL for the path")?;
            let items: Option<Retained<AnyObject>> =
                msg_send![class!(NSArray), arrayWithObject: &*url];
            let items = items.ok_or("no list of items")?;
            let allocated: Allocated<AnyObject> =
                msg_send![class!(UIActivityViewController), alloc];
            let sheet: Option<Retained<AnyObject>> = msg_send![
                allocated,
                initWithActivityItems: &*items,
                applicationActivities: std::ptr::null::<AnyObject>()
            ];
            let sheet = sheet.ok_or("the share sheet could not be made")?;

            // Saved to Files or sent, the file has been copied by the time the sheet closes.
            let folder: Option<PathBuf> = path.parent().map(Path::to_path_buf);
            let done = RcBlock::new(
                move |_activity: *mut AnyObject,
                      _completed: Bool,
                      _returned: *mut AnyObject,
                      _error: *mut AnyObject| {
                    if let Some(folder) = &folder {
                        let _ = std::fs::remove_dir_all(folder);
                    }
                },
            );
            let _: () = msg_send![&*sheet, setCompletionWithItemsHandler: &*done];

            let presenter = top_view_controller().ok_or("no view controller to show it over")?;
            // On iPad the sheet is a popover, which must have a source view.
            let popover: *mut AnyObject = msg_send![&*sheet, popoverPresentationController];
            if !popover.is_null() {
                let view: *mut AnyObject = msg_send![presenter, view];
                let _: () = msg_send![popover, setSourceView: view];
            }
            let _: () = msg_send![
                presenter,
                presentViewController: &*sheet,
                animated: true,
                completion: std::ptr::null::<AnyObject>()
            ];
        }
        Ok(())
    }

    /// The topmost controller: if a sheet is already up, it must present the new one.
    unsafe fn top_view_controller() -> Option<*mut AnyObject> {
        unsafe {
            let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
            let window: *mut AnyObject = msg_send![app, keyWindow];
            if window.is_null() {
                return None;
            }
            let mut controller: *mut AnyObject = msg_send![window, rootViewController];
            if controller.is_null() {
                return None;
            }
            loop {
                let presented: *mut AnyObject = msg_send![controller, presentedViewController];
                if presented.is_null() {
                    return Some(controller);
                }
                controller = presented;
            }
        }
    }
}
