// SPDX-License-Identifier: MPL-2.0
//! The event stream over UniFFI (CORE-009): a listener hears lifecycle and
//! settings changes until its subscription is cancelled or dropped.
//!
//! One test function on purpose: the core is a process-wide singleton.

use std::sync::{Arc, Mutex};

use pa_ffi::{
    CoreConfig, CoreDirs, CoreEvent, CoreEventListener, CoreLifecycle, core_enter_background,
    core_events_subscribe, core_init, core_shutdown, settings_export, settings_import,
    settings_remove, settings_set,
};

#[derive(Default)]
struct Recorder(Mutex<Vec<CoreEvent>>);

impl CoreEventListener for Recorder {
    fn on_event(&self, event: CoreEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Recorder {
    fn take(&self) -> Vec<CoreEvent> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn lifecycle(state: CoreLifecycle) -> CoreEvent {
    CoreEvent::Lifecycle { state }
}

fn settings(keys: &[&str]) -> CoreEvent {
    CoreEvent::SettingsChanged {
        keys: keys.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn listener_hears_lifecycle_and_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = |p: &str| tmp.path().join(p).to_string_lossy().into_owned();
    let config = CoreConfig {
        dirs: Some(CoreDirs {
            data: path("files"),
            config: path("files"),
            cache: path("cache"),
        }),
        worker_threads: Some(1),
        start_in_background: false,
    };

    let recorder = Arc::new(Recorder::default());
    let subscription = core_events_subscribe(recorder.clone());

    // Without app callbacks the bus still hears the lifecycle.
    core_init(config.clone(), None).unwrap();
    settings_set("ui.theme".into(), r#""dark""#.into()).unwrap();
    let backup = settings_export().unwrap();
    assert!(settings_remove("ui.theme".into()).unwrap());
    // Removing a missing key changes nothing: no event.
    assert!(!settings_remove("ui.theme".into()).unwrap());
    settings_import(backup, false).unwrap();
    core_enter_background().unwrap();
    core_shutdown().unwrap();

    assert_eq!(
        recorder.take(),
        vec![
            lifecycle(CoreLifecycle::Foreground),
            settings(&["ui.theme"]),
            settings(&["ui.theme"]),
            settings(&["ui.theme"]),
            lifecycle(CoreLifecycle::Background),
            lifecycle(CoreLifecycle::ShutDown),
        ]
    );

    assert!(subscription.cancel());
    assert!(!subscription.cancel());

    // A new run of the core: the cancelled listener hears nothing, a new
    // one hears events until its handle is dropped.
    let second = Arc::new(Recorder::default());
    let handle = core_events_subscribe(second.clone());
    core_init(config, None).unwrap();
    drop(handle);
    settings_set("ui.theme".into(), r#""light""#.into()).unwrap();
    core_shutdown().unwrap();

    assert!(recorder.take().is_empty(), "no events after cancel");
    assert_eq!(second.take(), vec![lifecycle(CoreLifecycle::Foreground)]);
}
