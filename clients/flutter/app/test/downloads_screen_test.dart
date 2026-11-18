// SPDX-License-Identifier: MPL-2.0
//
// UI-018: the Downloads screen. Progress follows the core's event bus
// (`core_events`, CORE-009) — job events of other kinds (Twitch) are
// ignored; pause / resume all; the space bar and the low-space warning;
// auto-cleanup on by default (applied once) and «Delete watched» now; the
// phone, desktop and TV layouts (remote focus); the «Downloads» entry of
// the sidebar and of the phone's Home.
//
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-018 \
//     flutter test test/downloads_screen_test.dart

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_offline.dart';
import 'package:kinolune/offline/offline_downloads.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/downloads_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/widgets/cards.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart' as core;
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 14, 20);

/// The fake queue, with its changes coming from core events instead of
/// its own stream: what [CoreOfflineBackend] does with `core_events`.
class _BusBackend extends FakeOfflineBackend {
  _BusBackend({super.titles});

  final bus = StreamController<core.CoreEvent>.broadcast();
  int reads = 0;

  @override
  Future<List<OfflineDownload>> downloads() {
    reads++;
    return super.downloads();
  }

  @override
  Stream<OfflineChange> changes() => offlineChanges(bus.stream);

  void job(
    int id,
    core.CoreJobChange change, {
    String kind = '',
    String? state,
    int done = 0,
    int? total,
  }) => bus.add(
    core.CoreEvent(
      domain: core.CoreEventDomain.jobChanged,
      libraryPath: '/data/library.db',
      jobChanged: core.CoreJobEvent(
        jobId: id,
        change: change,
        kind: kind,
        state: state,
        bytesDone: done,
        bytesTotal: total,
      ),
    ),
  );
}

Future<(AppServices, FakeOfflineBackend, MemoryPreferences)> _services({
  FakeOfflineBackend? backend,
  MemoryPreferences? prefs,
}) async {
  final b =
      backend ??
      FakeOfflineBackend(
        titles: {
          'm1': 'Arrival',
          'e1': 'Dark · S1 · E1',
          'e2': 'Dark · S1 · E2',
        },
      );
  final p = prefs ?? MemoryPreferences();
  final s = AppServices(
    library: FakeLibrary.items(const []),
    secureStore: MemorySecureStore(),
    preferences: p,
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
    offline: OfflineController(b),
  );
  await s.init();
  return (s, b, p);
}

Future<void> _open(
  WidgetTester t,
  AppServices s, {
  FormFactor formFactor = FormFactor.desktop,
}) async {
  await t.pumpWidget(testApp(s, formFactor: formFactor));
  await settle(t, 4);
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // Not awaited: the route stays open.
  // ignore: unawaited_futures
  nav.push(MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()));
  await settle(t, 6);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

Finder _key(String k) => find.byKey(ValueKey(k));

void main() {
  setUpAll(loadAppFonts);

  group('core events', () {
    core.CoreEvent ev(core.CoreJobEvent j) => core.CoreEvent(
      domain: core.CoreEventDomain.jobChanged,
      libraryPath: '',
      jobChanged: j,
    );

    test('job events of offline copies become changes', () {
      final p = offlineChangeOf(
        ev(
          const core.CoreJobEvent(
            jobId: 7,
            change: core.CoreJobChange.progress,
            kind: '',
            progress: .5,
            bytesDone: 50,
            bytesTotal: 100,
          ),
        ),
      )!;
      expect(p.kind, OfflineChangeKind.progress);
      expect((p.jobId, p.bytesDone, p.bytesTotal), (7, 50, 100));

      final st = offlineChangeOf(
        ev(
          const core.CoreJobEvent(
            jobId: 7,
            change: core.CoreJobChange.stateChanged,
            kind: offlineJobKind,
            state: 'waiting_resources',
            bytesDone: 0,
          ),
        ),
      )!;
      expect(st.kind, OfflineChangeKind.state);
      expect(st.state, OfflineState.waiting);

      expect(
        offlineChangeOf(
          ev(
            const core.CoreJobEvent(
              jobId: 8,
              change: core.CoreJobChange.added,
              kind: offlineJobKind,
              state: 'queued',
              bytesDone: 0,
            ),
          ),
        )!.kind,
        OfflineChangeKind.reload,
      );
      expect(
        offlineChangeOf(
          ev(
            const core.CoreJobEvent(
              jobId: 8,
              change: core.CoreJobChange.removed,
              kind: '',
              bytesDone: 0,
            ),
          ),
        )!.kind,
        OfflineChangeKind.removed,
      );
    });

    test('other jobs and other domains are ignored', () {
      expect(
        offlineChangeOf(
          ev(
            const core.CoreJobEvent(
              jobId: 9,
              change: core.CoreJobChange.stateChanged,
              kind: 'twitch.vod_download',
              state: 'running',
              bytesDone: 0,
            ),
          ),
        ),
        isNull,
      );
      expect(
        offlineChangeOf(
          const core.CoreEvent(
            domain: core.CoreEventDomain.libraryChanged,
            libraryPath: '',
          ),
        ),
        isNull,
      );
    });
  });

  testWidgets('progress and states follow the core events', (t) async {
    final backend = _BusBackend(titles: {'m1': 'Arrival'});
    final (s, _, _) = await _services(backend: backend);
    await s.offline!.download(const OfflineTarget.item('m1'));
    setSurface(t, const Size(1280, 860));
    await _open(t, s);
    expect(find.textContaining('Queued'), findsOneWidget);

    // Bytes of the job: the row moves on without reading the list.
    final reads = backend.reads;
    backend.job(
      100,
      core.CoreJobChange.progress,
      done: 256 << 20,
      total: 1 << 30,
    );
    await settle(t, 2);
    expect(
      t.widget<KProgressBar>(_key('download-j100-progress')).value,
      closeTo(.25, .01),
    );
    expect(find.textContaining('256 MB of 1 GB'), findsOneWidget);
    expect(backend.reads, reads);

    // A Twitch job in the same queue changes nothing.
    backend
      ..job(5, core.CoreJobChange.added, kind: 'twitch.vod_download')
      ..job(5, core.CoreJobChange.progress, done: 1, total: 2)
      ..job(5, core.CoreJobChange.removed);
    await settle(t, 4);
    expect(backend.reads, reads);

    // A state change shows at once; the list is then read for details.
    backend.job(
      100,
      core.CoreJobChange.stateChanged,
      kind: offlineJobKind,
      state: 'running',
    );
    // Before the reload (150 ms later), which brings the fake's own state.
    await t.pump(const Duration(milliseconds: 20));
    await t.pump(const Duration(milliseconds: 20));
    expect(find.textContaining('Downloading'), findsWidgets);
    await settle(t, 4);
    expect(backend.reads, greaterThan(reads));
    backend.job(
      100,
      core.CoreJobChange.progress,
      done: 768 << 20,
      total: 1 << 30,
    );
    await settle(t, 2);
    expect(
      t.widget<KProgressBar>(_key('download-j100-progress')).value,
      closeTo(.75, .01),
    );
    await _close(t);
  });

  testWidgets('pause all and resume all', (t) async {
    final (s, backend, _) = await _services();
    for (final id in ['m1', 'e1', 'e2']) {
      await s.offline!.download(OfflineTarget.item(id));
    }
    setSurface(t, const Size(1280, 860));
    await _open(t, s);
    expect(_key('downloads-queue-header'), findsOneWidget);
    await t.tap(_key('downloads-pause-all'));
    await settle(t, 4);
    expect(
      backend.items.map((d) => d.state),
      everyElement(OfflineState.paused),
    );
    expect(_key('downloads-pause-all'), findsNothing);
    await t.tap(_key('downloads-resume-all'));
    await settle(t, 4);
    expect(
      backend.items.map((d) => d.state),
      everyElement(OfflineState.queued),
    );
    await _close(t);
  });

  testWidgets('space, auto-cleanup and delete watched now', (t) async {
    final (s, backend, prefs) = await _services();
    // Auto-cleanup is on by default, once.
    expect(s.offline!.settings.deleteWatchedAfterDays, 7);
    expect(await prefs.read(Preferences.offlineCleanupDefault), '1');
    backend
      ..addCopy('m1', 'Arrival', 3 << 30, watched: true)
      ..addCopy('e1', 'Dark · S1 · E1', 1 << 30, watched: true)
      ..addCopy('e2', 'Dark · S1 · E2', 1 << 30);
    await s.offline!.refresh();
    setSurface(t, const Size(1280, 860));
    await _open(t, s);
    expect(_key('downloads-wide'), findsOneWidget);
    expect(_key('downloads-copies-header'), findsOneWidget);
    expect(find.textContaining('3 copies · 5 GB'), findsOneWidget);
    expect(_key('downloads-space-bar'), findsOneWidget);
    expect(find.text('Downloads · 5 GB'), findsOneWidget);
    expect(find.text('Other files · 70 GB'), findsOneWidget);
    expect(find.text('Free · 45 GB'), findsOneWidget);
    expect(_key('downloads-space-low'), findsNothing);
    expect(
      t.widget<Text>(_key('downloads-cleanup-rule')).data,
      contains('7 days'),
    );

    await t.tap(_key('downloads-delete-watched-now'));
    await settle(t, 4);
    expect(_key('downloads-delete-watched-dialog'), findsOneWidget);
    await t.tap(_key('downloads-delete-watched-confirm'));
    await settle(t, 6);
    expect(backend.items.map((d) => d.itemId), ['e2']);
    expect(find.text('2 copies deleted'), findsOneWidget);
    expect(_key('downloads-delete-watched-now'), findsNothing);

    // «Never» is kept: the default is not applied again.
    await s.offline!.setDeleteWatched(null);
    await settle(t, 4);
    expect(
      t.widget<Text>(_key('downloads-cleanup-rule')).data,
      contains('off'),
    );
    await _close(t);

    final (again, _, _) = await _services(backend: backend, prefs: prefs);
    expect(again.offline!.settings.deleteWatchedAfterDays, isNull);
  });

  test('copies from before UI-018 keep their rule (no default)', () async {
    final backend = FakeOfflineBackend(titles: {'m1': 'Arrival'})
      ..addCopy('m1', 'Arrival', 1 << 30, watched: true);
    final (s, _, prefs) = await _services(backend: backend);
    expect(s.offline!.settings.deleteWatchedAfterDays, isNull);
    expect(await prefs.read(Preferences.offlineCleanupDefault), '1');
    expect(backend.items.map((d) => d.itemId), ['m1']);
  });

  test('little space: under 2 GB or 5 % of the volume', () {
    expect(DownloadsSpaceCard.lowSpace(1 << 30, 64 << 30), isTrue);
    expect(DownloadsSpaceCard.lowSpace(4 << 30, 128 << 30), isTrue);
    expect(DownloadsSpaceCard.lowSpace(45 << 30, 120 << 30), isFalse);
  });

  testWidgets('a phone has one column', (t) async {
    final (s, _, _) = await _services();
    await s.offline!.download(const OfflineTarget.item('m1'));
    setSurface(t, const Size(390, 844), dpr: 2);
    await _open(t, s, formFactor: FormFactor.phone);
    expect(_key('downloads-narrow'), findsOneWidget);
    expect(_key('downloads-space'), findsOneWidget);
    expect(_key('download-j100-pause'), findsOneWidget);
    expect(t.takeException(), isNull);
    await _close(t);
  });

  testWidgets('a TV starts with focus on the list and moves with the remote', (
    t,
  ) async {
    final (s, _, _) = await _services();
    await s.offline!.download(const OfflineTarget.item('m1'));
    await s.offline!.download(const OfflineTarget.item('e1'));
    setSurface(t, const Size(960, 540), dpr: 2);
    await _open(t, s, formFactor: FormFactor.tv);
    expect(_key('downloads-wide'), findsOneWidget);
    bool focusedIn(String k) {
      final ctx = FocusManager.instance.primaryFocus?.context;
      if (ctx == null) return false;
      var found = false;
      ctx.visitAncestorElements((e) {
        if (e.widget.key == ValueKey(k)) found = true;
        return !found;
      });
      return found;
    }

    expect(focusedIn('downloads-pause-all'), isTrue);
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    expect(
      focusedIn('download-j101-pause') || focusedIn('download-j100-pause'),
      isTrue,
    );
    await pressKey(t, LogicalKeyboardKey.arrowRight);
    expect(
      focusedIn('download-j101-cancel') || focusedIn('download-j100-cancel'),
      isTrue,
    );
    expect(t.takeException(), isNull);
    await _close(t);
  });

  testWidgets('the sidebar and the phone Home open Downloads', (t) async {
    final (s, backend, _) = await _services();
    setSurface(t, const Size(1280, 800));
    await t.pumpWidget(testApp(s));
    await settle(t, 4);
    // Nothing downloaded: no entry.
    expect(_key('nav-downloads'), findsNothing);
    await s.offline!.download(const OfflineTarget.item('m1'));
    backend.advance(.5);
    await settle(t, 4);
    expect(_key('nav-downloads'), findsOneWidget);
    expect(find.textContaining('50'), findsWidgets);
    await t.tap(_key('nav-downloads'));
    await settle(t, 6);
    expect(_key('downloads-page'), findsOneWidget);
    await _close(t);

    setSurface(t, const Size(390, 844), dpr: 2);
    await t.pumpWidget(testApp(s, formFactor: FormFactor.phone));
    await settle(t, 6);
    expect(_key('nav-downloads'), findsOneWidget);
    await t.tap(_key('nav-downloads'));
    await settle(t, 6);
    expect(_key('downloads-page'), findsOneWidget);
    await _close(t);
  });

  // docs/screenshots/progress/ui-018 (KINOLUNE_SCREENSHOTS_DIR), in
  // Russian: the screen on a phone, a desktop and a TV.
  group('screenshots', () {
    for (final (name, ff, size, dpr) in [
      ('phone', FormFactor.phone, const Size(390, 844), 2.0),
      ('desktop', FormFactor.desktop, const Size(1280, 800), 1.0),
      ('tv', FormFactor.tv, const Size(960, 540), 2.0),
    ]) {
      testWidgets('downloads on $name', (t) async {
        final backend = FakeOfflineBackend(
          titles: {
            'm1': 'Прибытие',
            'e3': 'Тьма · S1 · E3',
            'e4': 'Тьма · S1 · E4',
            'e5': 'Тьма · S1 · E5',
          },
        )..size = 3 << 30;
        final (s, _, _) = await _services(backend: backend);
        s.locale.value = const Locale('ru');
        backend
          ..addCopy('m9', 'Дюна: Часть вторая', 7 << 30, watched: true)
          ..addCopy('e1', 'Тьма · S1 · E1', 1288490188, watched: true)
          ..addCopy('e2', 'Тьма · S1 · E2', 1288490188);
        for (final id in ['m1', 'e3', 'e4', 'e5']) {
          await s.offline!.download(OfflineTarget.item(id));
        }
        backend.advance(.62);
        final queued = s.offline!.queue;
        await backend.control(
          queued.firstWhere((d) => d.itemId == 'e4'),
          OfflineAction.pause,
        );
        await s.offline!.refresh();
        setSurface(t, size, dpr: dpr);
        await _open(t, s, formFactor: ff);
        expect(t.takeException(), isNull);
        await capture(t, '01-downloads-$name');
        if (name == 'desktop') {
          await _close(t);
          await t.pumpWidget(testApp(s));
          await settle(t, 6);
          await capture(t, '02-sidebar-entry-desktop');
        }
        if (name == 'phone') {
          await t.scrollUntilVisible(
            _key('settings-section-downloads'),
            300,
            scrollable: find.descendant(
              of: _key('downloads-narrow'),
              matching: find.byType(Scrollable),
            ),
          );
          await settle(t, 4);
          await capture(t, '02-downloads-phone-settings');
        }
        await _close(t);
      });
    }
  });
}
