//! The Mac waking from sleep (macOS): the network may have come back
//! meanwhile, possibly with the window already focused.

use std::sync::{Mutex, Once};

use iced::{
    Subscription,
    futures::{Stream, channel::mpsc},
};
use objc2::{AnyThread, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
use objc2_foundation::{NSNotification, NSObject};

define_class!(
    /// Hears the workspace's wake notifications.
    #[unsafe(super(NSObject))]
    #[name = "MtrxWakeObserver"]
    struct Observer;

    impl Observer {
        #[unsafe(method(didWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            if let Some(sender) = WOKE.lock().unwrap().as_ref() {
                let _ = sender.unbounded_send(());
            }
        }
    }
);

/// Where wake notifications send, to [`woke`].
static WOKE: Mutex<Option<mpsc::UnboundedSender<()>>> = Mutex::new(None);

/// Each time the Mac wakes from sleep.
pub fn woke() -> Subscription<()> {
    Subscription::run(receive)
}

fn receive() -> impl Stream<Item = ()> {
    static OBSERVE: Once = Once::new();
    OBSERVE.call_once(|| {
        let observer: Retained<Observer> = unsafe { msg_send![Observer::alloc(), init] };
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: `didWake:` takes the notification; the observer lives as
        // long as the app.
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(didWake:),
                Some(NSWorkspaceDidWakeNotification),
                None,
            );
        }
        std::mem::forget(observer);
    });
    let (sender, receiver) = mpsc::unbounded();
    *WOKE.lock().unwrap() = Some(sender);
    receiver
}
