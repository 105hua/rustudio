//! macOS integration: the menu bar's app menu, and files opened from Finder.
//!
//! **App menu.** winit's default app menu quits with `terminate:`, which
//! ends the process at once: no "Discard unsaved changes?" and no saved
//! layout. Here Quit (⌘Q) closes the window instead (`performClose:`), the
//! same path as its close button, so the editor asks first. The menu has only
//! the standard Hide / Hide Others / Show All / Quit items: any other key
//! equivalent here would take that key from the editor (⌘M is Studio's
//! Import).
//!
//! **Opening files.** Finder (double-click, "Open With", a file dropped on
//! the Dock icon) doesn't put the file in argv: it sends an Open Documents
//! Apple Event, which winit ignores. [`install_open_handler`] takes that event
//! before the app finishes launching; the editor picks the files up with
//! [`take_opened`] each frame.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSObject, NSString};

/// Replaces the menu bar's app menu. Call on the main thread once the app
/// has finished launching (from the eframe app's constructor).
pub fn install_menu() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let item = |title: &str, action: Option<Sel>, key: &str| -> Retained<NSMenuItem> {
        // SAFETY: standard NSResponder / NSApplication action selectors.
        unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                action,
                &NSString::from_str(key),
            )
        }
    };
    let menu = NSMenu::new(mtm);
    menu.addItem(&item("Hide rustudio", Some(sel!(hide:)), "h"));
    let others = item("Hide Others", Some(sel!(hideOtherApplications:)), "h");
    others.setKeyEquivalentModifierMask(NSEventModifierFlags::Option | NSEventModifierFlags::Command);
    menu.addItem(&others);
    menu.addItem(&item("Show All", Some(sel!(unhideAllApplications:)), ""));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item("Quit rustudio", Some(sel!(performClose:)), "q"));

    let app_item = NSMenuItem::new(mtm);
    app_item.setSubmenu(Some(&menu));
    let bar = NSMenu::new(mtm);
    bar.addItem(&app_item);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
}

/// Files Finder asked to open, not yet taken.
static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// Wakes the editor when a file comes in while it is idle.
static WAKE: OnceLock<egui::Context> = OnceLock::new();

/// Files Finder asked to open since the last call.
pub fn take_opened() -> Vec<PathBuf> {
    OPENED.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

/// The editor's context, to repaint (and so take the files) on arrival.
pub fn set_wake(ctx: &egui::Context) {
    let _ = WAKE.set(ctx.clone());
}

const fn four(c: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*c)
}
const K_CORE_EVENT_CLASS: u32 = four(b"aevt");
const K_AE_OPEN_DOCUMENTS: u32 = four(b"odoc");
const KEY_DIRECT_OBJECT: u32 = four(b"----");

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no ivars.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RustudioOpenHandler"]
    struct OpenHandler;

    impl OpenHandler {
        /// NSApplicationWillFinishLaunchingNotification: AppKit has put its
        /// own Apple Event handlers in by now, so this one replaces its
        /// Open Documents handler (installing it earlier would be undone).
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _n: &NSNotification) {
            // SAFETY: self answers handleOpen:withReply: (below).
            unsafe {
                let _: () = msg_send![
                    &*NSAppleEventManager::sharedAppleEventManager(),
                    setEventHandler: self as &AnyObject,
                    andSelector: sel!(handleOpen:withReply:),
                    forEventClass: K_CORE_EVENT_CLASS,
                    andEventID: K_AE_OPEN_DOCUMENTS
                ];
            }
        }

        #[unsafe(method(handleOpen:withReply:))]
        fn handle_open(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            // SAFETY: paramDescriptorForKeyword: takes an AEKeyword (u32).
            let list: Option<Retained<NSAppleEventDescriptor>> =
                unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
            let Some(list) = list else { return };
            let mut paths = Vec::new();
            // (1-based; a single file may come as the descriptor itself)
            let n = list.numberOfItems();
            let items: Vec<Retained<NSAppleEventDescriptor>> = if n > 0 {
                (1..=n).filter_map(|i| list.descriptorAtIndex(i)).collect()
            } else {
                vec![list]
            };
            for d in items {
                if let Some(p) = d.fileURLValue().and_then(|u| u.path()) {
                    paths.push(PathBuf::from(p.to_string()));
                }
            }
            if paths.is_empty() {
                return;
            }
            if let Ok(mut v) = OPENED.lock() {
                v.extend(paths);
            }
            if let Some(ctx) = WAKE.get() {
                ctx.request_repaint();
            }
        }
    }
);

/// Takes Finder's Open Documents events from launch on. Call on the main
/// thread before the event loop starts.
pub fn install_open_handler() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: plain NSObject subclass init.
    let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::alloc(mtm), init] };
    // SAFETY: the handler answers willFinishLaunching:; it lives for the
    // whole process (leaked below), as the notification centre and the
    // Apple Event manager don't retain it.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &handler,
            sel!(willFinishLaunching:),
            Some(&NSString::from_str("NSApplicationWillFinishLaunchingNotification")),
            None,
        );
    }
    std::mem::forget(handler);
}
