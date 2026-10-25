//! Log records and panics to NSLog, so the Xcode console shows them.

use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

struct NsLogLogger;

impl log::Log for NsLogLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            nslog(&format!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            ));
        }
    }

    fn flush(&self) {}
}

pub fn install() {
    let _ = log::set_logger(&NsLogLogger).map(|()| log::set_max_level(log::LevelFilter::Info));
    std::panic::set_hook(Box::new(|info| nslog(&format!("flint panic: {info}"))));
}

fn nslog(message: &str) {
    unsafe extern "C" {
        fn NSLog(format: *mut AnyObject, ...);
    }
    let Ok(message) = std::ffi::CString::new(message.replace('\0', "")) else {
        return;
    };
    // The SSH threads have no autorelease pool of their own.
    autoreleasepool(|_| unsafe {
        let format: *mut AnyObject =
            msg_send![class!(NSString), stringWithUTF8String: c"%@".as_ptr()];
        let message: *mut AnyObject =
            msg_send![class!(NSString), stringWithUTF8String: message.as_ptr()];
        NSLog(format, message);
    });
}
