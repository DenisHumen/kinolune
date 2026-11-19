// SPDX-License-Identifier: MPL-2.0
//
// QA-016: live e2e scenarios. The REAL app (`main()`: Rust core, libmpv,
// the network) starts on an empty profile and is clicked through against
// the SMB share of the e2e stand (tools/qa/e2e/stand.sh): first run → the
// share (found on the LAN, else by address) → scan → home → recognition
// (with PA_TMDB_API_KEY only) → a film: play 10 s, seek, stop →
// «Продолжить» → a series: the next episode → music → search → settings →
// diagnostics «Сохранить отчёт». Every step asserts real state (item
// counts, positions, the player clock moving) and leaves a screenshot.
// Optional soak: PA_E2E_SOAK=N cycles of browse → play → seek → back with
// memory, stalls and player errors.
//
// Not for CI: run through tools/qa/e2e/run.sh, which sets up the stand,
// Xvfb and the environment:
//   PA_E2E_SMB_HOST, PA_E2E_SMB_USER, PA_E2E_SMB_PASS  the stand
//   PA_E2E_OUT        where screenshots go
//   PA_E2E_SOAK       soak cycles (0: none)
//   PA_TMDB_API_KEY   optional; the recognition check is skipped without it
// The app's data dirs (XDG_*) must point at an empty directory.
//
// Lines starting with `E2E ` are the machine-readable log the runner
// turns into the report.

import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:kinolune/diagnostics/diagnostics.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/main.dart' as app;
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/screens/onboarding_screen.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show PlayRequest, PlayerScreen, PlayerScreenState;

final _env = Platform.environment;
final _out = _env['PA_E2E_OUT'] ?? 'build/e2e';
final _host = _env['PA_E2E_SMB_HOST'] ?? 'kinolune-e2e';
final _user = _env['PA_E2E_SMB_USER'] ?? '';
final _pass = _env['PA_E2E_SMB_PASS'] ?? '';
final _soak = int.tryParse(_env['PA_E2E_SOAK'] ?? '') ?? 0;

/// BUG-001 bisect: what a soak cycle opens. `app` (default) — the film
/// page and «Смотреть», everything the app does; `no-trickplay` — the same
/// with seek previews off; `player-smb` — the bare player on the item's
/// own URI (no film page, previews or segments); `player-local` — the bare
/// player on the file of the stand's library on the local disk
/// (PA_E2E_LIBRARY).
final _soakMode = _env['PA_E2E_SOAK_MODE'] ?? 'app';
final _library = _env['PA_E2E_LIBRARY'] ?? '';
final _tmdb = _env['PA_TMDB_API_KEY'] ?? '';

/// The real films of the stand (tools/qa/e2e/library.py); titles as the
/// core parses them from the file names.
const _realFilms = [
  'Big Buck Bunny',
  'Sintel',
  'Tears of Steel',
  'Elephants Dream',
  'Cosmos Laundromat',
];

var _shot = 0;
final _scanClock = Stopwatch();
final _failures = <String>[];

void _log(String line) {
  // ignore: avoid_print
  print('E2E $line');
}

int _rssMb() => ProcessInfo.currentRss ~/ (1024 * 1024);

/// BUG-001: where native memory sits — threads of the process and glibc
/// malloc (`mallinfo2`: from the system, in use, mmapped), to tell a leak
/// (in use grows) from fragmentation (only from-the-system grows).
String _nativeMem() {
  final parts = <String>[];
  try {
    final status = File('/proc/self/status').readAsLinesSync();
    final threads = status.firstWhere((l) => l.startsWith('Threads:'));
    parts.add('threads=${threads.split(RegExp(r'\s+')).last}');
  } catch (_) {}
  final info = _mallinfo2;
  if (info != null) {
    final m = info();
    const mb = 1024 * 1024;
    parts
      ..add('heap=${(m.arena + m.hblkhd) ~/ mb}MB')
      ..add('inuse=${(m.uordblks + m.hblkhd) ~/ mb}MB')
      ..add('free=${m.fordblks ~/ mb}MB');
  }
  return parts.join(' ');
}

final class _MallInfo2 extends ffi.Struct {
  @ffi.Size()
  external int arena;
  @ffi.Size()
  external int ordblks;
  @ffi.Size()
  external int smblks;
  @ffi.Size()
  external int hblks;
  @ffi.Size()
  external int hblkhd;
  @ffi.Size()
  external int usmblks;
  @ffi.Size()
  external int fsmblks;
  @ffi.Size()
  external int uordblks;
  @ffi.Size()
  external int fordblks;
  @ffi.Size()
  external int keepcost;
}

final _MallInfo2 Function()? _mallinfo2 = () {
  if (!Platform.isLinux) return null;
  try {
    return ffi.DynamicLibrary.process()
        .lookupFunction<_MallInfo2 Function(), _MallInfo2 Function()>(
          'mallinfo2',
        );
  } catch (_) {
    return null;
  }
}();

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.fullyLive;

  testWidgets('live: the real app against the SMB stand', (tester) async {
    expect(_user, isNotEmpty, reason: 'PA_E2E_SMB_USER is not set');
    Directory('$_out/screenshots').createSync(recursive: true);
    final t = _Driver(tester);
    _log('NOTE start rss=${_rssMb()}MB host=$_host soak=$_soak');

    await t.step('first run: onboarding', critical: true, () async {
      unawaited(app.main(const []));
      await t.waitFor(find.byType(OnboardingScreen), 'onboarding', secs: 60);
      return 'onboarding shown on an empty profile';
    });

    await t.step('add the SMB source', critical: true, () async {
      final l = t.l10n;
      for (var i = 0; i < 3; i++) {
        await t.tap(find.byKey(const ValueKey('onboarding-next')));
        await t.idle(600);
      }
      await t.waitFor(find.text(l.onboardingSourceTitle), 'source step');
      // NET-020: shares on the LAN first. The stand announces itself over
      // mDNS; give the browse a few seconds.
      final discovered = find.textContaining(
        RegExp('kinolune-e2e|Kinolune e2e', caseSensitive: false),
      );
      final found = await t.waitUntil(
        () => discovered.evaluate().isNotEmpty,
        secs: 15,
      );
      String how;
      if (found) {
        how = 'found on the LAN';
        await t.tap(discovered.first);
      } else {
        how = 'not found on the LAN, by address';
        await t.tap(find.text(l.sourcesTypeSmb));
      }
      await t.idle(1500);
      await t.shot('smb-dialog');
      final server = find.byKey(const ValueKey('smb-server'));
      if (server.evaluate().isNotEmpty) {
        final ctl = tester.widget<TextField>(server).controller;
        if (ctl == null || ctl.text.isEmpty) {
          await tester.enterText(server, _host);
        }
        await t.tap(find.byKey(const ValueKey('smb-next')));
      }
      await t.waitFor(find.byKey(const ValueKey('smb-user')), 'sign-in form');
      await tester.enterText(find.byKey(const ValueKey('smb-user')), _user);
      await tester.enterText(find.byKey(const ValueKey('smb-password')), _pass);
      await t.tap(find.byKey(const ValueKey('smb-connect')));
      await t.waitFor(
        find.byKey(const ValueKey('smb-share-media')),
        'share list',
        secs: 30,
      );
      await t.shot('smb-shares');
      await t.tap(find.byKey(const ValueKey('smb-share-media')));
      await t.waitFor(
        find.byKey(const ValueKey('smb-use-folder')),
        'folder view',
      );
      await t.tap(find.byKey(const ValueKey('smb-use-folder')));
      _scanClock.start();
      await t.waitUntil(() => t.services.sources.isNotEmpty, secs: 30);
      expect(t.services.sources, isNotEmpty, reason: 'no source was added');
      return '$how; source ${t.services.sources.single.path}';
    });

    await t.step('scan the share', critical: true, () async {
      final s = t.services;
      await t.waitUntil(
        () => s.scanProgress?.isDone == true && !s.scanning,
        secs: 600,
      );
      final p = s.scanProgress;
      _scanClock.stop();
      expect(p?.isDone, isTrue, reason: 'scan did not finish in 10 min');
      await t.shot('scan-done');
      return 'files found ${p!.filesFound}, added + scanned in '
          '${_scanClock.elapsed.inMilliseconds} ms'
          '${p.error == null ? '' : '; error: ${p.error}'}';
    });

    await t.step('finish onboarding, home fills', critical: true, () async {
      final l = t.l10n;
      await t.tap(find.byKey(const ValueKey('onboarding-next')));
      await t.idle(800);
      await t.tap(find.text(l.onboardingFinish));
      await t.waitFor(find.byType(AppShell), 'app shell', secs: 30);
      final lib = t.services.library;
      final movies = await lib.movies().first;
      final shows = await lib.shows().first;
      final tracks = await lib.tracks().first;
      await t.idle(3000);
      await t.shot('home');
      expect(movies.length, greaterThanOrEqualTo(10), reason: 'movies');
      expect(shows.length, greaterThanOrEqualTo(3), reason: 'shows');
      expect(tracks.length, greaterThanOrEqualTo(10), reason: 'tracks');
      for (final title in _realFilms) {
        expect(
          movies.any(
            (m) => m.title.toLowerCase().contains(title.toLowerCase()),
          ),
          isTrue,
          reason: '$title is not in the library',
        );
      }
      return 'movies ${movies.length}, shows ${shows.length}, '
          'tracks ${tracks.length}; all 5 real films listed';
    });

    await t.step('recognition (TMDB)', () async {
      if (_tmdb.isEmpty) {
        _log('SKIP recognition: no PA_TMDB_API_KEY in the environment');
        return 'skipped (no key)';
      }
      await t.services.setTmdbKey(_tmdb);
      await t.services.scan();
      await t.waitUntil(() => !t.services.scanning, secs: 600);
      final ok = await t.waitUntilAsync(() async {
        final movies = await t.services.library.movies().first;
        return movies.where((m) => m.tmdbId != null).length >= 3;
      }, secs: 300);
      final movies = await t.services.library.movies().first;
      final matched = movies.where((m) => m.tmdbId != null).toList();
      await t.goTo(Section.home);
      await t.shot('recognised');
      expect(ok, isTrue, reason: 'fewer than 3 films matched');
      return 'matched ${matched.length}/${movies.length}';
    });

    // ------------------------------------------------------------ a film
    late String filmId;
    await t.step('open a film card', critical: true, () async {
      await t.goTo(Section.movies);
      final card = find.textContaining('Big Buck Bunny');
      await t.waitFor(card, 'Big Buck Bunny card', secs: 30);
      await t.tap(card.first);
      await t.waitFor(find.byType(MovieDetailScreen), 'film page');
      await t.idle(1500);
      await t.shot('film-page');
      final movies = await t.services.library.movies().first;
      filmId = movies.firstWhere((m) => m.title.contains('Big Buck Bunny')).id;
      return 'film page of $filmId';
    });

    await t.step('play 10 s', critical: true, () async {
      await t.tap(find.byKey(detailPlayKey));
      final player = await t.player();
      final r = await t.watchPlayback(player, const Duration(seconds: 10));
      await t.shot('playing');
      expect(r.advanced.inMilliseconds, greaterThan(7000), reason: '$r');
      return r.toString();
    });

    await t.step('seek', critical: true, () async {
      final player = await t.player();
      final before = player.engine.state.value.position;
      // The remote's fast-forward key (no Linux key code in the test
      // keymaps: sent with Android's).
      await tester.sendKeyEvent(
        LogicalKeyboardKey.mediaFastForward,
        platform: 'android',
      );
      await t.idle(1500);
      final afterKey = player.engine.state.value.position;
      const target = Duration(minutes: 3);
      await player.engine.seek(target);
      final r = await t.watchPlayback(player, const Duration(seconds: 5));
      await t.shot('after-seek');
      expect(
        player.engine.state.value.position,
        greaterThanOrEqualTo(target),
        reason: 'position after the seek',
      );
      expect(r.advanced.inMilliseconds, greaterThan(3000), reason: '$r');
      return 'fast-forward key ${_fmt(before)} → ${_fmt(afterKey)}; '
          'seek to 3:00, then $r';
    });

    await t.step('stop (Esc), progress saved', critical: true, () async {
      final player = await t.player();
      final at = player.engine.state.value.position;
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      final closed = await t.waitUntil(
        () => find.byType(PlayerScreen).evaluate().isEmpty,
        secs: 10,
      );
      if (!closed) {
        // A second Esc: the first may only hide the OSD.
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await t.waitUntil(
          () => find.byType(PlayerScreen).evaluate().isEmpty,
          secs: 10,
        );
      }
      expect(find.byType(PlayerScreen), findsNothing, reason: 'player open');
      final p = await t.services.library.progress(filmId);
      expect(p, isNotNull, reason: 'no saved progress');
      final saved = Duration(milliseconds: p!.positionMs);
      expect(
        (saved - at).inSeconds.abs(),
        lessThan(15),
        reason: 'saved ${_fmt(saved)} vs stopped at ${_fmt(at)}',
      );
      return 'stopped at ${_fmt(at)}, saved ${_fmt(saved)}';
    });

    await t.step('«Продолжить» shows the position', () async {
      await t.goTo(Section.home);
      final l = t.l10n;
      await t.waitFor(
        find.textContaining(l.homeContinueWatching),
        'Continue row',
        secs: 20,
      );
      final row = await t.services.library.continueWatching().first;
      await t.idle(1500);
      await t.shot('continue-row');
      final hit = row.where((e) => e.$1.id == filmId).toList();
      expect(hit, isNotEmpty, reason: 'film not in Continue');
      return 'Continue: ${row.length} item(s); film at '
          '${_fmt(Duration(milliseconds: hit.first.$2.positionMs))}';
    });

    // ----------------------------------------------------------- a series
    await t.step('series: the next episode', () async {
      final shows = await t.services.library.shows().first;
      final show = shows.firstWhere((s) => s.title.contains('Pioneer One'));
      final eps = await t.services.library.episodes(show.id).first;
      eps.sort(
        (a, b) => ((a.season ?? 0) * 1000 + (a.episode ?? 0)).compareTo(
          (b.season ?? 0) * 1000 + (b.episode ?? 0),
        ),
      );
      expect(eps.length, greaterThanOrEqualTo(2), reason: 'episodes');
      final next = await t.services.library.nextEpisode(eps.first.id);
      expect(next?.id, eps[1].id, reason: 'core nextEpisode');
      await t.goTo(Section.shows);
      final card = find.textContaining('Pioneer One');
      await t.waitFor(card, 'show card', secs: 20);
      await t.tap(card.first);
      await t.waitFor(find.byType(ShowDetailScreen), 'show page');
      await t.idle(1500);
      await t.shot('show-page');
      await t.tap(find.byKey(showEpisodeKey(eps.first.id)));
      final player = await t.player();
      await t.watchPlayback(player, const Duration(seconds: 3));
      final dur = player.engine.state.value.duration;
      final first = player.request.title;
      await player.engine.seek(dur - const Duration(seconds: 4));
      // Up Next starts the next episode at the end (video requests carry
      // no item id: the title changes to the next episode's).
      final moved = await t.waitUntil(
        () =>
            find.byType(PlayerScreen).evaluate().isNotEmpty &&
            t.tester
                    .state<PlayerScreenState>(find.byType(PlayerScreen))
                    .request
                    .title !=
                first,
        secs: 40,
      );
      await t.shot('next-episode');
      expect(moved, isTrue, reason: 'next episode did not start');
      final p2 = await t.player();
      final second = p2.request.title;
      final r = await t.watchPlayback(p2, const Duration(seconds: 4));
      expect(r.advanced.inMilliseconds, greaterThan(2000), reason: '$r');
      await t.closePlayer();
      // The progress of the new episode is its own (the Up Next saver).
      final p = await t.services.library.progress(eps[1].id);
      expect(p, isNotNull, reason: 'no progress saved for E02');
      return '«$first» (${_fmt(dur)}) → «$second» by Up Next; $r; '
          'E02 saved at ${_fmt(Duration(milliseconds: p!.positionMs))}';
    });

    // -------------------------------------------------------------- music
    await t.step('music: play a track', () async {
      await t.goTo(Section.music);
      await t.idle(1500);
      final tracks = await t.services.library.tracks().first;
      final track = tracks.firstWhere(
        (x) => x.title.contains('Red'),
        orElse: () => tracks.first,
      );
      await t.shot('music');
      final music = t.services.music;
      // The album page: by its title on the music home/library.
      final album = find.textContaining('Colour Bars');
      if (album.evaluate().isNotEmpty) {
        await t.tap(album.first);
        await t.idle(1500);
      }
      final row = find.byKey(ValueKey('track-${track.id}'));
      String how;
      if (row.evaluate().isNotEmpty) {
        await t.tap(row.first);
        how = 'tapped the track row';
      } else {
        await t.tap(find.text('Red').first);
        how = 'tapped «Red»';
      }
      final started = await t.waitUntil(
        () => music.playing && music.position > const Duration(seconds: 1),
        secs: 20,
      );
      final a = music.position;
      await t.idle(3000);
      final b = music.position;
      await t.shot('music-playing');
      final title = music.current?.title ?? '?';
      await music.stop();
      expect(started, isTrue, reason: 'music did not start');
      expect((b - a).inMilliseconds, greaterThan(2000), reason: '$a → $b');
      return '$how; «$title» ${_fmt(a)} → ${_fmt(b)}';
    });

    // ------------------------------------------------------------- search
    await t.step('search', () async {
      await t.goTo(Section.search);
      await t.idle(800);
      final field = find.byType(TextField).first;
      await tester.enterText(field, 'Sintel');
      final hit = find.textContaining('Sintel');
      final ok = await t.waitUntil(
        () => hit.evaluate().length >= 2, // the query + a result
        secs: 15,
      );
      await t.shot('search');
      final core = await t.services.library.search('sintel');
      expect(ok, isTrue, reason: 'no result on screen');
      expect(core, isNotEmpty, reason: 'core search');
      return 'core: ${core.length} result(s), shown on screen';
    });

    // ----------------------------------------------------------- settings
    await t.step('settings', () async {
      await t.goTo(Section.settings);
      await t.idle(1200);
      await t.shot('settings');
      final adv = find.byKey(const ValueKey('settings-advanced'));
      if (adv.evaluate().isEmpty) {
        _log('SKIP settings basic/advanced switch: not in this build (UI-041)');
        return 'settings open; no basic/advanced switch yet';
      }
      await t.tap(adv);
      await t.idle(800);
      await t.shot('settings-advanced');
      return 'switched to advanced';
    });

    await t.step('diagnostics «Сохранить отчёт»', () async {
      final started = DateTime.now().subtract(const Duration(seconds: 2));
      final export = find.byKey(const ValueKey('settings-diagnostics-export'));
      final ok = await t.scrollTo(export);
      expect(ok, isTrue, reason: 'no export tile');
      await t.tap(export);
      final saved = find.byKey(const ValueKey('diagnostics-saved'));
      final snack = find.byType(SnackBar);
      await t.waitUntil(
        () => saved.evaluate().isNotEmpty || snack.evaluate().isNotEmpty,
        secs: 30,
      );
      if (saved.evaluate().isEmpty) {
        await t.shot('report-not-saved');
        final said = snack.evaluate().isEmpty
            ? 'nothing on screen'
            : find
                  .descendant(of: snack, matching: find.byType(Text))
                  .evaluate()
                  .map((e) => (e.widget as Text).data)
                  .join(' ');
        throw TestFailure(
          'no «report saved» dialog in 30 s; $said; '
          'diagnostics available: ${Diagnostics.instance.available}',
        );
      }
      await t.shot('report-saved');
      final text = find
          .descendant(of: saved, matching: find.byType(Text))
          .evaluate()
          .map((e) => (e.widget as Text).data ?? '')
          .join(' ');
      // The zip goes to Downloads, Documents or (Linux without
      // xdg-user-dirs) the home folder; on the runner the home folder is
      // the host's: find it, measure it, remove it.
      final home = Platform.environment['HOME'] ?? '/nonexistent';
      final zips = <File>[
        for (final dir in {home, '$home/Downloads', '$home/Documents'})
          if (Directory(dir).existsSync())
            ...Directory(dir).listSync().whereType<File>().where(
              (f) =>
                  f.path.contains('kinolune-diagnostics-') &&
                  f.path.endsWith('.zip') &&
                  f.statSync().modified.isAfter(started),
            ),
      ];
      final sizes = [for (final z in zips) z.lengthSync()];
      for (final z in zips) {
        z.deleteSync();
      }
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.idle(500);
      return '«$text»; zip '
          '${sizes.isEmpty ? 'not found' : '${sizes.first} bytes (removed)'}';
    });

    // --------------------------------------------------------------- soak
    if (_soak > 0) await _runSoak(t);

    _log('MEM end rss=${_rssMb()}MB');
    if (_failures.isNotEmpty) {
      fail('${_failures.length} step(s) failed: ${_failures.join('; ')}');
    }
  }, timeout: const Timeout(Duration(hours: 6)));
}

Future<void> _runSoak(_Driver t) async {
  final lib = t.services.library;
  final movies = await lib.movies().first;
  final films = [
    for (final title in _realFilms)
      movies.firstWhere((m) => m.title.contains(title)),
  ];
  final rss = <int>[];
  var stalls = 0, errors = 0, failed = 0;
  final bare = _soakMode.startsWith('player-');
  if (_soakMode == 'no-trickplay') {
    await t.services.setTrickplayGenerate(false);
  }
  final local = <String, String>{
    if (_soakMode == 'player-local')
      for (final f in films) f.id: _localFile(f.title),
  };
  _log('NOTE soak mode $_soakMode');
  final sw = Stopwatch()..start();
  for (var i = 1; i <= _soak; i++) {
    final film = films[i % films.length];
    final c = Stopwatch()..start();
    try {
      if (bare) {
        final uri = local[film.id] ?? film.playbackUri;
        final services = t.services;
        unawaited(
          Navigator.of(t.tester.element(find.byType(AppShell)), rootNavigator: true)
              .push(
                MaterialPageRoute<void>(
                  builder: (_) => PlayerScreen(
                    request: PlayRequest(uri: uri, title: film.title),
                    onProgress: (position, duration) {},
                    engineFactory: services.playerEngineFactory,
                  ),
                ),
              ),
        );
      } else {
        // Browse: the film through Search (the grid only builds the
        // visible cards).
        await t.goTo(Section.search);
        await t.tester.enterText(find.byType(TextField).first, film.title);
        final result = find.byKey(ValueKey('search-${film.id}'));
        await t.waitFor(result, 'search result ${film.title}', secs: 20);
        await t.tap(result);
        await t.waitFor(find.byType(MovieDetailScreen), 'film page');
        await t.tap(find.byKey(detailPlayKey));
      }
      final player = await t.player();
      final a = await t.watchPlayback(player, const Duration(seconds: 6));
      final dur = player.engine.state.value.duration;
      // Forward or back from where it resumed: a seek lands near the
      // target and plays on from there.
      final target = Duration(
        milliseconds: (dur.inMilliseconds * (0.2 + (i % 7) / 10)).round(),
      );
      await player.engine.seek(target);
      final b = await t.watchPlayback(player, const Duration(seconds: 4));
      stalls += a.stalls + b.stalls;
      final error = player.engine.state.value.error;
      if (error != null) {
        errors++;
        _log('NOTE soak cycle $i player error: ${_oneLine(error)}');
      }
      await t.tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.waitUntil(
        () => find.byType(PlayerScreen).evaluate().isEmpty,
        secs: 10,
      );
      if (find.byType(PlayerScreen).evaluate().isNotEmpty) {
        await t.tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await t.idle(1000);
      }
      await t.goTo(Section.home);
      await t.idle(500);
      final ok =
          a.advanced.inMilliseconds > 3000 &&
          (b.end - target).inMilliseconds > 2000 &&
          (b.end - target).inSeconds < 10;
      if (!ok) failed++;
      rss.add(_rssMb());
      _log(
        'SOAK cycle $i ${ok ? 'ok' : 'FAIL'} ${film.title} '
        '${c.elapsed.inSeconds}s rss=${rss.last}MB ${_nativeMem()} '
        'play=[$a] seek=[$b]',
      );
    } catch (e) {
      failed++;
      _log('SOAK cycle $i FAIL ${film.title}: ${_oneLine(e)}');
      await t.shot('soak-fail-$i');
      // Back to a known place.
      for (var k = 0; k < 3; k++) {
        if (find.byType(PlayerScreen).evaluate().isEmpty) break;
        await t.tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await t.idle(800);
      }
    }
  }
  if (rss.isNotEmpty) {
    final first = rss.first, last = rss.last;
    final max = rss.reduce((a, b) => a > b ? a : b);
    _log(
      'MEM soak first=${first}MB last=${last}MB max=${max}MB '
      'growth=${last - first}MB over ${rss.length} cycles',
    );
  }
  _log(
    'SOAK done cycles=$_soak failed=$failed stalls=$stalls '
    'player_errors=$errors in ${sw.elapsed.inMinutes} min',
  );
  if (failed > 0) _failures.add('soak: $failed cycle(s) failed');
}

/// The file of the stand's film [title] under PA_E2E_LIBRARY.
String _localFile(String title) {
  String norm(String s) =>
      s.toLowerCase().replaceAll(RegExp('[^a-z0-9]+'), ' ').trim();
  final want = norm(title);
  final file = Directory(_library)
      .listSync(recursive: true)
      .whereType<File>()
      .firstWhere(
        (f) =>
            norm(f.uri.pathSegments.last).startsWith(want) &&
            RegExp(r'\.(mkv|mov|mp4)$').hasMatch(f.path),
      );
  return file.path;
}

String _fmt(Duration d) {
  final s = d.inSeconds;
  return '${s ~/ 60}:${(s % 60).toString().padLeft(2, '0')}';
}

String _oneLine(Object e) => '$e'.split('\n').first;

/// What the player did while it was watched.
class _Watch {
  _Watch(this.start, this.end, this.wall, this.stalls, this.buffering);
  final Duration start, end, wall;
  final int stalls, buffering;
  Duration get advanced => end - start;

  @override
  String toString() =>
      '${_fmt(start)} → ${_fmt(end)} in ${wall.inMilliseconds / 1000}s '
      'wall, stalls $stalls, buffering samples $buffering';
}

class _Driver {
  _Driver(this.tester);
  final WidgetTester tester;

  AppServices get services =>
      Services.read(tester.element(find.byType(MaterialApp).first));

  AppLocalizations get l10n =>
      AppLocalizations.of(tester.element(find.byType(Scaffold).first));

  Future<void> idle([int ms = 300]) async {
    await Future<void>.delayed(Duration(milliseconds: ms));
    await tester.pump();
  }

  Future<bool> waitUntil(bool Function() ok, {int secs = 20}) async {
    final end = DateTime.now().add(Duration(seconds: secs));
    while (DateTime.now().isBefore(end)) {
      await tester.pump();
      if (ok()) return true;
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
    await tester.pump();
    return ok();
  }

  Future<bool> waitUntilAsync(
    Future<bool> Function() ok, {
    int secs = 20,
  }) async {
    final end = DateTime.now().add(Duration(seconds: secs));
    while (DateTime.now().isBefore(end)) {
      await tester.pump();
      if (await ok()) return true;
      await Future<void>.delayed(const Duration(seconds: 2));
    }
    return ok();
  }

  Future<void> waitFor(Finder f, String what, {int secs = 20}) async {
    if (!await waitUntil(() => f.evaluate().isNotEmpty, secs: secs)) {
      throw TestFailure('timed out after ${secs}s waiting for $what');
    }
  }

  Future<void> tap(Finder f) async {
    await waitFor(f, f.describeMatch(Plurality.one));
    await tester.ensureVisible(f.first);
    await tester.pump();
    await tester.tap(f.first, warnIfMissed: false);
    await idle(400);
  }

  Future<bool> scrollTo(Finder f) async {
    if (f.evaluate().isNotEmpty) return true;
    final list = find.byType(Scrollable);
    if (list.evaluate().isEmpty) return false;
    for (var i = 0; i < 30 && f.evaluate().isEmpty; i++) {
      await tester.drag(list.first, const Offset(0, -400), warnIfMissed: false);
      await idle(200);
    }
    return f.evaluate().isNotEmpty;
  }

  /// A section of the shell by its icon (rail, sidebar or tabs).
  /// Closes the player with Esc, as a user would (twice at most).
  Future<void> closePlayer() async {
    for (var k = 0; k < 3; k++) {
      if (find.byType(PlayerScreen).evaluate().isEmpty) return;
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await waitUntil(
        () => find.byType(PlayerScreen).evaluate().isEmpty,
        secs: 8,
      );
    }
  }

  Future<void> goTo(Section s) async {
    await closePlayer();
    // Pages pushed inside a section (a film, a show, an album): back.
    for (var k = 0; k < 5; k++) {
      final back = find.byIcon(Icons.arrow_back);
      if (back.evaluate().isEmpty) break;
      await tap(back);
      await idle(400);
    }
    final shell = find.byType(AppShell, skipOffstage: false);
    for (var k = 0; k < 10; k++) {
      final nav = Navigator.of(tester.element(shell));
      if (!nav.canPop()) break;
      nav.pop();
      await idle(500);
    }
    final icon = find.byIcon(s.icon);
    final selected = find.byIcon(s.selectedIcon);
    if (icon.evaluate().isNotEmpty) {
      await tap(icon);
    } else if (selected.evaluate().isNotEmpty) {
      await tap(selected);
    } else {
      throw TestFailure('no navigation entry for ${s.name}');
    }
    await idle(800);
  }

  Future<PlayerScreenState> player() async {
    await waitFor(find.byType(PlayerScreen), 'the player', secs: 30);
    final state = tester.state<PlayerScreenState>(find.byType(PlayerScreen));
    final ok = await waitUntil(
      () =>
          state.engine.state.value.playing &&
          state.engine.state.value.position > Duration.zero,
      secs: 45,
    );
    if (!ok) {
      final s = state.engine.state.value;
      throw TestFailure(
        'player did not start: playing=${s.playing} '
        'buffering=${s.buffering} error=${s.error}',
      );
    }
    return state;
  }

  /// Samples the player every 250 ms for [span]: a stall is a second or
  /// more without the clock moving while it should be playing.
  Future<_Watch> watchPlayback(PlayerScreenState p, Duration span) async {
    final start = p.engine.state.value.position;
    final sw = Stopwatch()..start();
    var last = start;
    var lastMove = DateTime.now();
    var stalls = 0, buffering = 0;
    var inStall = false;
    while (sw.elapsed < span) {
      await idle(250);
      final s = p.engine.state.value;
      if (s.buffering) buffering++;
      if (s.position != last) {
        last = s.position;
        lastMove = DateTime.now();
        inStall = false;
      } else if (!inStall &&
          DateTime.now().difference(lastMove) > const Duration(seconds: 1)) {
        stalls++;
        inStall = true;
      }
    }
    return _Watch(
      start,
      p.engine.state.value.position,
      sw.elapsed,
      stalls,
      buffering,
    );
  }

  Future<void> step(
    String name,
    Future<String> Function() body, {
    bool critical = false,
  }) async {
    final sw = Stopwatch()..start();
    try {
      final detail = await body();
      _log('STEP ok "$name" ${sw.elapsed.inMilliseconds}ms: $detail');
    } catch (e, st) {
      _log('FAIL "$name" ${sw.elapsed.inMilliseconds}ms: ${_oneLine(e)}');
      debugPrint('$st');
      await shot('FAIL-${name.replaceAll(RegExp(r'[^A-Za-z0-9]+'), '-')}');
      _failures.add(name);
      if (!critical) {
        try {
          await closePlayer();
        } catch (_) {}
      }
      if (critical) {
        fail('critical step "$name" failed: ${_oneLine(e)}');
      }
    }
  }

  /// The X screen (with the video) through ffmpeg x11grab; the Flutter
  /// layer tree when that is not available.
  Future<void> shot(String name) async {
    await tester.pump();
    _shot++;
    final file =
        '$_out/screenshots/${_shot.toString().padLeft(2, '0')}-$name.png';
    final display = Platform.environment['DISPLAY'];
    if (display != null) {
      final r = await Process.run('ffmpeg', [
        '-loglevel', 'error', '-y', '-f', 'x11grab', //
        '-i', display, '-frames:v', '1', file,
      ]);
      if (r.exitCode == 0) return;
    }
    try {
      final view = tester.binding.renderViews.first;
      final layer = view.debugLayer! as OffsetLayer;
      final size = tester.view.physicalSize;
      final img = await layer.toImage(Offset.zero & size);
      final png = await img.toByteData(format: ui.ImageByteFormat.png);
      img.dispose();
      File(file).writeAsBytesSync(png!.buffer.asUint8List());
    } catch (e) {
      _log('NOTE screenshot $name failed: ${_oneLine(e)}');
    }
  }
}
