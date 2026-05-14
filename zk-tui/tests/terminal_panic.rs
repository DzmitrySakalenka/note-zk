#![allow(unused_crate_dependencies)]

use std::{
    panic,
    sync::{Arc, Mutex},
    thread,
};

use zk_tui::terminal::{
    install_panic_hook,
    test_support::{install_restore_recorder, terminal_guard_for_test},
};

static HOOK_LOCK: Mutex<()> = Mutex::new(());

fn restore_events(events: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
    events
        .lock()
        .expect("restore event list should lock")
        .clone()
}

#[test]
fn drop_runs_restore_routine() {
    let _hook_lock = HOOK_LOCK.lock().expect("panic hook test lock should lock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let _recorder = install_restore_recorder(Arc::clone(&events));

    {
        let _guard = terminal_guard_for_test();
    }

    assert_eq!(restore_events(&events), vec!["restore"]);
}

#[test]
fn panic_hook_restores_before_previous_hook_and_unwinds_guard() {
    let _hook_lock = HOOK_LOCK.lock().expect("panic hook test lock should lock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let _recorder = install_restore_recorder(Arc::clone(&events));

    let original_hook = panic::take_hook();
    let previous_events = Arc::clone(&events);
    panic::set_hook(Box::new(move |_| {
        previous_events
            .lock()
            .expect("restore event list should lock")
            .push("previous");
    }));

    install_panic_hook();
    let panic_result = thread::spawn(|| {
        let _guard = terminal_guard_for_test();
        panic!("terminal panic hook test");
    })
    .join();

    let _installed_hook = panic::take_hook();
    panic::set_hook(original_hook);

    assert!(panic_result.is_err(), "thread should propagate panic");
    assert_eq!(
        restore_events(&events),
        vec!["restore", "previous", "restore"]
    );
}
