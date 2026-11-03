//! Finder integration: receive "open document" Apple Events (double-click,
//! "Open With", drag onto the Dock icon) and forward them to the inbox.
//!
//! winit's application delegate ignores these events, so we install our own
//! handler with NSAppleEventManager when the app is about to finish launching —
//! the point at which AppKit has registered its defaults and before the first
//! `odoc` event is dispatched.

use crate::ipc::{Inbox, OpenRequest};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AllocAnyThread};
use objc2_foundation::{NSAppleEventDescriptor, NSNotification, NSNotificationCenter, NSString};
use std::path::PathBuf;
use std::sync::OnceLock;

static INBOX: OnceLock<Inbox> = OnceLock::new();

const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const K_AE_OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "CryocodeOpenHandler"]
    struct OpenHandler;

    impl OpenHandler {
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _note: &NSNotification) {
            install_apple_event_handler(self);
        }

        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            for path in document_paths(event) {
                if let Some(inbox) = INBOX.get() {
                    inbox.push(OpenRequest { path, line: None, col: None });
                }
            }
        }
    }
);

fn install_apple_event_handler(handler: &OpenHandler) {
    unsafe {
        let manager: *mut AnyObject = msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager];
        let _: () = msg_send![
            manager,
            setEventHandler: handler,
            andSelector: sel!(handleOpenDocuments:withReplyEvent:),
            forEventClass: K_CORE_EVENT_CLASS,
            andEventID: K_AE_OPEN_DOCUMENTS
        ];
    }
}

fn document_paths(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        let list: Option<Retained<NSAppleEventDescriptor>> = msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT];
        let Some(list) = list else { return out };
        let n = list.numberOfItems();
        let items: Vec<Retained<NSAppleEventDescriptor>> = if n == 0 {
            vec![list]
        } else {
            (1..=n).filter_map(|i| list.descriptorAtIndex(i)).collect()
        };
        for item in items {
            if let Some(url) = item.fileURLValue() {
                if let Some(p) = url.path() {
                    out.push(PathBuf::from(p.to_string()));
                }
            }
        }
    }
    out
}

/// Must run on the main thread before the event loop starts.
pub fn install(inbox: Inbox) {
    let _ = INBOX.set(inbox);
    let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::alloc(), init] };
    unsafe {
        let center = NSNotificationCenter::defaultCenter();
        let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
        center.addObserver_selector_name_object(&handler, sel!(willFinishLaunching:), Some(&name), None);
    }
    // The handler lives for the whole process.
    std::mem::forget(handler);
}
