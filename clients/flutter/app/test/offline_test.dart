// SPDX-License-Identifier: MPL-2.0
//
// NET-019: offline downloads in the app over an in-memory queue
// (FakeOfflineBackend): the Download button of a network movie with the
// version picker and its states, no button for local files, the
// Downloads screen following progress events with pause and delete, the
// settings (Wi-Fi only kept in preferences, delete watched) and the
// network the queue is told.

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_offline.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/offline/network_watch.dart';
import 'package:kinolune/offline/offline_downloads.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/downloads_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/cards.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 14, 20);

const _uhd = 'smb://nas/media/Movies/Arrival (2016)/Arrival.2016.2160p.mkv';
const _hd = 'smb://nas/media/Movies/Arrival (2016)/Arrival.2016.1080p.mkv';

final _arrival = LibraryItem(
  id: 'm1',
  kind: ItemKind.movie,
  title: 'Arrival',
  year: 2016,
  filePath: _uhd,
  streamUri: 'http://127.0.0.1:1/t/1/Movies/Arrival.2016.2160p.mkv',
  addedAt: _now,
  versionCount: 2,
);

final _local = LibraryItem(
  id: 'm2',
  kind: ItemKind.movie,
  title: 'Local Film',
  year: 1999,
  filePath: '/media/Movies/local.mkv',
  addedAt: _now,
);

Future<(AppServices, FakeOfflineBackend, MemoryPreferences)> _services() async {
  final lib = FakeLibrary.items(
    [_arrival, _local],
    versions: {
      'm1': const [
        MediaVersion(
          id: '11',
          label: '4K',
          resolution: '4K',
          filePath: _uhd,
          sizeBytes: 40 << 30,
          isDefault: true,
        ),
        MediaVersion(
          id: '12',
          label: '1080p',
          resolution: '1080p',
          filePath: _hd,
          sizeBytes: 8 << 30,
        ),
      ],
    },
  );
  final backend = FakeOfflineBackend(titles: {'m1': 'Arrival'});
  final prefs = MemoryPreferences();
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    preferences: prefs,
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
    offline: OfflineController(backend),
  );
  await s.init();
  return (s, backend, prefs);
}

Future<void> _openMovie(WidgetTester t, AppServices s, String title) async {
  setSurface(t, const Size(1280, 800));
  await t.pumpWidget(testApp(s, section: Section.movies));
  await settle(t, 10);
  await t.tap(find.text(title).first);
  await settle(t, 10);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('a network movie downloads the chosen version and shows it', (
    t,
  ) async {
    final (s, backend, _) = await _services();
    await _openMovie(t, s, 'Arrival');
    final button = find.byKey(const ValueKey('offline-button'));
    expect(button, findsOneWidget);
    expect(
      find.descendant(of: button, matching: find.text('Download')),
      findsOneWidget,
    );
    await t.tap(button);
    await settle(t, 4);
    expect(find.byKey(const ValueKey('offline-version-dialog')), findsOne);
    await t.tap(find.byKey(const ValueKey('offline-version-12')));
    await settle(t, 6);
    expect(backend.requested, hasLength(1));
    expect(backend.requested.single.id, 'm1');
    expect(backend.requested.single.versionFileId, 12);
    expect(find.text('1 download queued'), findsOneWidget);

    backend.advance(.45);
    await settle(t, 4);
    expect(
      find.descendant(of: button, matching: find.textContaining('45')),
      findsOneWidget,
    );
    backend.advance(1);
    await settle(t, 4);
    expect(
      find.descendant(of: button, matching: find.text('Downloaded')),
      findsOneWidget,
    );

    // Downloaded: the button deletes after a confirmation.
    await t.tap(button);
    await settle(t, 4);
    await t.tap(find.byKey(const ValueKey('offline-delete-confirm')));
    await settle(t, 6);
    expect(backend.items, isEmpty);
    expect(
      find.descendant(of: button, matching: find.text('Download')),
      findsOneWidget,
    );
    await _close(t);
  });

  testWidgets('a local file has no Download button', (t) async {
    final (s, _, _) = await _services();
    await _openMovie(t, s, 'Local Film');
    expect(find.byKey(const ValueKey('offline-button')), findsNothing);
    await _close(t);
  });

  testWidgets('the Downloads screen follows progress, pauses and deletes', (
    t,
  ) async {
    final (s, backend, prefs) = await _services();
    await s.offline!.download(const OfflineTarget.item('m1'));
    setSurface(t, const Size(1280, 900));
    await t.pumpWidget(testApp(s));
    await settle(t, 4);
    final nav = t.state<NavigatorState>(find.byType(Navigator).first);
    // Not awaited: the route stays open.
    // ignore: unawaited_futures
    nav.push(MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('downloads-page')), findsOneWidget);
    expect(find.text('Arrival'), findsOneWidget);
    expect(find.textContaining('Queued'), findsOneWidget);

    backend.advance(.25);
    await settle(t, 4);
    final bar = t.widget<KProgressBar>(
      find.byKey(const ValueKey('download-j100-progress')),
    );
    expect(bar.value, closeTo(.25, .01));
    expect(find.textContaining('256 MB of 1 GB'), findsOneWidget);

    await t.tap(find.byKey(const ValueKey('download-j100-pause')));
    await settle(t, 4);
    expect(backend.items.single.state, OfflineState.paused);
    expect(find.byKey(const ValueKey('download-j100-resume')), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('download-j100-resume')));
    await settle(t, 4);
    backend.advance(1);
    await settle(t, 6);
    expect(find.textContaining('On this device'), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('download-c100-delete')));
    await settle(t, 4);
    await t.tap(find.byKey(const ValueKey('offline-delete-confirm')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('downloads-empty')), findsOneWidget);

    // Settings on the same screen: Wi-Fi only is kept in preferences.
    final wifi = find.byKey(const ValueKey('settings-downloads-wifi'));
    expect(
      t
          .widget<Switch>(
            find.descendant(of: wifi, matching: find.byType(Switch)),
          )
          .value,
      isTrue,
    );
    await t.tap(wifi);
    await settle(t, 4);
    expect(await prefs.read(Preferences.offlineWifiOnly), 'false');
    expect((await backend.settings()).wifiOnly, isFalse);
    await _close(t);
  });

  test('the queue gets the network the platform reports', () {
    expect(
      offlineNetworkOf([ConnectivityResult.wifi]),
      OfflineNetwork.unmetered,
    );
    expect(
      offlineNetworkOf([ConnectivityResult.mobile]),
      OfflineNetwork.metered,
    );
    expect(
      offlineNetworkOf([ConnectivityResult.mobile, ConnectivityResult.wifi]),
      OfflineNetwork.unmetered,
    );
    expect(offlineNetworkOf([ConnectivityResult.none]), OfflineNetwork.offline);
    expect(offlineNetworkOf(const []), OfflineNetwork.offline);
    expect(
      offlineNetworkOf([ConnectivityResult.ethernet]),
      OfflineNetwork.unmetered,
    );
  });

  // docs/screenshots/progress/net-019 (KINOLUNE_SCREENSHOTS_DIR), in
  // Russian: the movie page while downloading, the Downloads screen on a
  // desktop and a phone, the settings section.
  testWidgets('screenshots', (t) async {
    final (s, backend, _) = await _services();
    s.locale.value = const Locale('ru');
    backend
      ..size = 7 << 29
      ..titles.addAll({'e1': 'Тьма · S1 · E1', 'e2': 'Тьма · S1 · E2'});
    await s.offline!.download(const OfflineTarget.item('e1'));
    await s.offline!.download(const OfflineTarget.item('e2'));
    await s.offline!.download(const OfflineTarget.item('m1'));
    backend.advance(.42);
    await s.offline!.refresh();
    await _openMovie(t, s, 'Arrival');
    await capture(t, '01-movie-downloading');

    final nav = t.state<NavigatorState>(find.byType(Navigator).first);
    // ignore: unawaited_futures
    nav.push(MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()));
    await settle(t, 6);
    await capture(t, '02-downloads-desktop');
    await _close(t);

    final (p, pb, _) = await _services();
    p.locale.value = const Locale('ru');
    pb.titles.addAll({'e1': 'Тьма · S1 · E1', 'e2': 'Тьма · S1 · E2'});
    await p.offline!.download(const OfflineTarget.item('e1'));
    await p.offline!.download(const OfflineTarget.item('e2'));
    await p.offline!.download(const OfflineTarget.item('m1'));
    pb.advance(1);
    pb.advance(.3);
    await p.setOfflineWifiOnly(true);
    await p.offline!.setDeleteWatched(7);
    setSurface(t, const Size(390, 844), dpr: 2);
    await t.pumpWidget(testApp(p, formFactor: FormFactor.phone));
    await settle(t, 6);
    final pnav = t.state<NavigatorState>(find.byType(Navigator).first);
    // ignore: unawaited_futures
    pnav.push(MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()));
    await settle(t, 6);
    await capture(t, '03-downloads-phone');
    await _close(t);
  });
}
