//! macOS integration: the menu bar's app menu, and files opened from Finder.
//!
//! **Quitting.** winit's default app menu quits with `terminate:`, and so
//! does AppKit's answer to the Quit Apple Event (the Dock's Quit, logging
//! out, AppleScript): the process ends at once, with no "Discard unsaved
//! changes?" and no saved layout. Here both ask the window to close the way
//! its close button does (`windowShouldClose:`, which winit turns into a
//! close request), so the editor asks first; logging out waits for the answer
//! like with any app holding unsaved work. The menu has only
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
use std::sync::atomic::{AtomicPtr, Ordering};
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
    let quit = item("Quit rustudio", Some(sel!(quitRequested:)), "q");
    let handler = HANDLER.load(Ordering::Acquire);
    if !handler.is_null() {
        // SAFETY: the handler is leaked in install_open_handler: it lives
        // for the whole process.
        unsafe { quit.setTarget(Some(&*(handler as *const AnyObject))) };
    }
    menu.addItem(&quit);

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
const K_AE_QUIT_APPLICATION: u32 = four(b"quit");

/// The handler object (leaked at install), the Quit menu item's target.
static HANDLER: AtomicPtr<OpenHandler> = AtomicPtr::new(std::ptr::null_mut());

/// Asks the editor's window to close as its close button would; quits at
/// once only when there is no window.
fn request_close(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: NSApplication / NSWindow / NSWindowDelegate methods with their
    // documented argument and return types.
    unsafe {
        let mut win: Option<Retained<AnyObject>> = msg_send![&app, keyWindow];
        if win.is_none() {
            win = msg_send![&app, mainWindow];
        }
        if win.is_none() {
            let windows: Retained<AnyObject> = msg_send![&app, windows];
            win = msg_send![&windows, firstObject];
        }
        let Some(win) = win else {
            app.terminate(None);
            return;
        };
        let delegate: Option<Retained<AnyObject>> = msg_send![&win, delegate];
        match delegate {
            // winit's delegate answers NO and sends CloseRequested.
            Some(d) => {
                let _: bool = msg_send![&d, windowShouldClose: &*win];
            }
            None => {
                let _: () = msg_send![&win, performClose: std::ptr::null::<AnyObject>()];
            }
        }
    }
}
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
                let _: () = msg_send![
                    &*NSAppleEventManager::sharedAppleEventManager(),
                    setEventHandler: self as &AnyObject,
                    andSelector: sel!(handleQuit:withReply:),
                    forEventClass: K_CORE_EVENT_CLASS,
                    andEventID: K_AE_QUIT_APPLICATION
                ];
            }
        }

        /// The Quit Apple Event (the Dock's Quit, logging out, AppleScript).
        #[unsafe(method(handleQuit:withReply:))]
        fn handle_quit(&self, _event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            request_close(self.mtm());
        }

        /// The app menu's Quit (⌘Q).
        #[unsafe(method(quitRequested:))]
        fn quit_requested(&self, _sender: Option<&AnyObject>) {
            request_close(self.mtm());
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
    HANDLER.store(Retained::into_raw(handler), Ordering::Release);
}
