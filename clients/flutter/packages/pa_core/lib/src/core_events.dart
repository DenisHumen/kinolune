// SPDX-License-Identifier: MPL-2.0
// The app-wide event stream of the core (CORE-009, ADR-0121), written by
// hand around the generated calls (BUG-007).
import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show PlatformInt64;

import 'rust/api/events.dart';

/// All events of the core from the moment the stream is listened to, until
/// the subscription is cancelled. Any number of streams may be open; the
/// UI usually keeps one (ADR-0121).
///
/// Listening subscribes in the core before `listen` returns, so every event
/// of a call made after it arrives. Cancelling closes the stream in the core
/// right away: a plain flutter_rust_bridge stream would notice the cancel
/// only at its next event, and until then `cancel()` would not complete and
/// its open port would keep the isolate alive (BUG-007).
Stream<CoreEvent> coreEvents() {
  late final StreamController<CoreEvent> controller;
  PlatformInt64? id;
  StreamSubscription<CoreEvent>? inner;
  controller = StreamController<CoreEvent>(
    onListen: () {
      final key = id = coreEventsNextId();
      inner = coreEventsListen(id: key).listen(
        controller.add,
        onError: controller.addError,
        onDone: controller.close,
      );
    },
    onPause: () => inner?.pause(),
    onResume: () => inner?.resume(),
    onCancel: () {
      final key = id;
      if (key != null) coreEventsClose(id: key);
      // Completes once the core's end of stream arrives, at once.
      return inner?.cancel();
    },
  );
  return controller.stream;
}
