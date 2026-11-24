// SPDX-License-Identifier: MPL-2.0
// The long streams of the core (CORE-009, ADR-0121): the app-wide event
// stream, sync events, network discovery and offline downloads, written by
// hand around the generated calls (BUG-007, BUG-008).
import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show PlatformInt64;

import 'rust/api/discovery.dart';
import 'rust/api/events.dart';
import 'rust/api/offline.dart';
import 'rust/api/sync.dart';

/// A long stream of the core that [listen] opens under an id and a cancel
/// closes by that id.
///
/// Listening subscribes in the core before `listen` returns, so every event
/// of a call made after it arrives. Cancelling closes the stream in the core
/// right away: a plain flutter_rust_bridge stream would notice the cancel
/// only at its next event, and until then `cancel()` would not complete,
/// its open port would keep the isolate alive and the core would keep its
/// subscription (BUG-007, BUG-008).
Stream<T> closableStream<T>(Stream<T> Function(PlatformInt64 id) listen) {
  late final StreamController<T> controller;
  PlatformInt64? id;
  StreamSubscription<T>? inner;
  controller = StreamController<T>(
    onListen: () {
      final key = id = coreStreamNextId();
      final Stream<T> stream;
      try {
        stream = listen(key);
      } catch (e, st) {
        id = null;
        controller
          ..addError(e, st)
          ..close();
        return;
      }
      inner = stream.listen(
        controller.add,
        onError: controller.addError,
        onDone: controller.close,
      );
    },
    onPause: () => inner?.pause(),
    onResume: () => inner?.resume(),
    onCancel: () {
      final key = id;
      if (key != null) coreStreamClose(id: key);
      // Completes once the core's end of stream arrives, at once.
      return inner?.cancel();
    },
  );
  return controller.stream;
}

/// All events of the core from the moment the stream is listened to, until
/// the subscription is cancelled. Any number of streams may be open; the
/// UI usually keeps one (ADR-0121). A cancel ends the stream at once
/// ([closableStream]).
Stream<CoreEvent> coreEvents() =>
    closableStream((id) => coreEventsListen(id: id));

/// Long streams of [CoreSync] (BUG-008).
extension CoreSyncStreams on CoreSync {
  /// Sync events until the service is dropped or the subscription is
  /// cancelled; `pb.*` of other profiles are left out. A cancel ends the
  /// stream at once ([closableStream]).
  Stream<CoreSyncEvent> events() =>
      closableStream((id) => eventsListen(id: id));
}

/// Long streams of [CoreDiscovery] (BUG-008).
extension CoreDiscoveryStreams on CoreDiscovery {
  /// The visible services: the current list at once, then the whole list
  /// again after every change, until [CoreDiscovery.close] or a cancel,
  /// which ends the stream at once ([closableStream]). A closed discovery
  /// gives an error (`closed`).
  Stream<List<CoreDiscoveredService>> watch() =>
      closableStream((id) => watchListen(id: id));
}

/// Long streams of [CoreOffline] (BUG-008).
extension CoreOfflineStreams on CoreOffline {
  /// Changes of offline downloads until the subscription is cancelled,
  /// which ends the stream at once ([closableStream]).
  Stream<CoreOfflineEvent> watch() =>
      closableStream((id) => watchListen(id: id));
}
