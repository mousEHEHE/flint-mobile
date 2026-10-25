//! On iOS GitHub's sign-in page is a sheet (ASWebAuthenticationSession), so the app stays in
//! front and its loopback port still gets GitHub's answer.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use gpui::App;

/// The loopback page sends the sheet here once GitHub answered; the sheet closes on it.
pub const LANDING: &str = "flint://signed-in";

/// `is_cancelled` is set when the user closes the page before signing in.
pub fn open(url: &str, is_cancelled: Arc<AtomicBool>, cx: &mut App) -> Result<(), &'static str> {
    #[cfg(target_os = "ios")]
    {
        let _ = cx;
        ios::start(url, is_cancelled)
    }
    #[cfg(not(target_os = "ios"))]
    {
        // Android and the preview use the browser, which reports no cancel: the port keeps waiting.
        let _ = is_cancelled;
        cx.open_url(url);
        Ok(())
    }
}

/// For a sign-in that ends some other way (error, time-out, cancel). Does nothing if closed.
pub fn close() {
    #[cfg(target_os = "ios")]
    ios::close();
}

#[cfg(target_os = "ios")]
mod ios {
    use std::cell::RefCell;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    use block2::RcBlock;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, ClassBuilder, Sel};
    use objc2::{class, msg_send, sel, MainThreadMarker};

    #[link(name = "AuthenticationServices", kind = "framework")]
    unsafe extern "C" {}

    /// Must match the scheme of `super::LANDING`.
    const SCHEME: &str = "flint";

    struct Running {
        session: Retained<AnyObject>,
        /// The session holds its presentation provider weakly.
        _anchor: Retained<AnyObject>,
    }

    thread_local! {
        /// Keeps the session alive while it runs; a finished one stays until the next start.
        static RUNNING: RefCell<Option<Running>> = const { RefCell::new(None) };
    }

    pub fn start(url: &str, is_cancelled: Arc<AtomicBool>) -> Result<(), &'static str> {
        if MainThreadMarker::new().is_none() {
            return Err("the sign-in sheet opens on the main thread only");
        }
        close();

        let page = ns_url(url).ok_or("GitHub's address is not a URL")?;
        let scheme = ns_string(SCHEME).ok_or("no string for the scheme")?;
        // Called once: with the flint:// URL, or with null when the user closed the sheet.
        let done = RcBlock::new(move |callback: *mut AnyObject, _error: *mut AnyObject| {
            if callback.is_null() {
                is_cancelled.store(true, Ordering::Relaxed);
            }
        });

        // SAFETY: the arguments are an NSURL, an NSString and a block of the handler's type.
        let session: Option<Retained<AnyObject>> = unsafe {
            let allocated: Allocated<AnyObject> =
                msg_send![class!(ASWebAuthenticationSession), alloc];
            msg_send![
                allocated,
                initWithURL: &*page,
                callbackURLScheme: &*scheme,
                completionHandler: &*done
            ]
        };
        let session = session.ok_or("the sign-in sheet could not be made")?;
        // SAFETY: `new` on an NSObject subclass.
        let anchor: Option<Retained<AnyObject>> = unsafe { msg_send![anchor_class(), new] };
        let anchor = anchor.ok_or("the sign-in sheet has no window")?;

        // SAFETY: the provider answers the protocol's one required method; the flag is a BOOL.
        let is_started: bool = unsafe {
            let _: () = msg_send![&*session, setPresentationContextProvider: &*anchor];
            // Not ephemeral, so a GitHub login in Safari carries over.
            let _: () = msg_send![&*session, setPrefersEphemeralWebBrowserSession: false];
            msg_send![&*session, start]
        };
        if !is_started {
            return Err("the sign-in sheet did not open");
        }
        RUNNING.with(|running| {
            *running.borrow_mut() = Some(Running {
                session,
                _anchor: anchor,
            })
        });
        Ok(())
    }

    pub fn close() {
        // Taken out first: cancelling it may call its completion handler at once.
        let Some(running) = RUNNING.with(|running| running.borrow_mut().take()) else {
            return;
        };
        // SAFETY: `cancel` takes no arguments and returns nothing; a finished session ignores it.
        unsafe {
            let _: () = msg_send![&*running.session, cancel];
        }
    }

    fn ns_string(text: &str) -> Option<Retained<AnyObject>> {
        let text = CString::new(text).ok()?;
        // SAFETY: a NUL-terminated UTF-8 string, copied before this returns.
        unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
    }

    fn ns_url(text: &str) -> Option<Retained<AnyObject>> {
        let text = ns_string(text)?;
        // SAFETY: an NSString argument.
        unsafe { msg_send![class!(NSURL), URLWithString: &*text] }
    }

    fn anchor_class() -> &'static AnyClass {
        static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
        CLASS.get_or_init(|| {
            let mut builder = ClassBuilder::new(c"FlintWebAuthAnchor", class!(NSObject))
                .expect("no other class is called FlintWebAuthAnchor");
            // SAFETY: the function takes and returns what the selector does.
            unsafe {
                builder.add_method(
                    sel!(presentationAnchorForWebAuthenticationSession:),
                    presentation_anchor
                        as unsafe extern "C" fn(
                            *mut AnyObject,
                            Sel,
                            *mut AnyObject,
                        ) -> *mut AnyObject,
                );
            }
            if let Some(protocol) =
                AnyProtocol::get(c"ASWebAuthenticationPresentationContextProviding")
            {
                builder.add_protocol(protocol);
            }
            builder.register()
        })
    }

    /// The key window, or the app's first one, returned unretained as the method expects.
    unsafe extern "C" fn presentation_anchor(
        _this: *mut AnyObject,
        _sel: Sel,
        _session: *mut AnyObject,
    ) -> *mut AnyObject {
        // SAFETY: UIApplication's getters, called on the main thread like this method.
        unsafe {
            let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
            let window: *mut AnyObject = msg_send![app, keyWindow];
            if !window.is_null() {
                return window;
            }
            let windows: *mut AnyObject = msg_send![app, windows];
            msg_send![windows, firstObject]
        }
    }
}
