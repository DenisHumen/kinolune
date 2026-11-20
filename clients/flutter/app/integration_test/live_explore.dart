// SPDX-License-Identifier: MPL-2.0
//
// QA-016, exploratory pass (PA_E2E_EXPLORE=1): after the scenarios, walk
// the screens the scenarios do not reach — every section, settings top to
// bottom, «Всё ли в порядке», recognition review, sources with a wrong
// password, search edge cases, Twitch without a login, downloads — and
// leave a screenshot plus the visible texts of each (`texts/<name>.txt`)
// for a human to read. Steps only assert that the screen opened; findings
// are read from the screenshots, the texts and the log.

part of 'live_test.dart';

final _explore = (_env['PA_E2E_EXPLORE'] ?? '') == '1';

/// Every visible string on the screen, one per line, to `texts/<name>.txt`.
void _dumpTexts(String name) {
  final seen = <String>{};
  for (final e in find.byType(RichText).evaluate()) {
    final s = (e.widget as RichText).text.toPlainText().trim();
    if (s.isNotEmpty) seen.add(s.replaceAll('\n', ' ⏎ '));
  }
  final dir = Directory('$_out/texts')..createSync(recursive: true);
  File('${dir.path}/$name.txt').writeAsStringSync('${seen.join('\n')}\n');
}

/// The library as recognition left it, to `recognition-<tag>.tsv`.
Future<String> _dumpRecognition(_Driver t, String tag) async {
  final lib = t.services.library;
  final movies = await lib.movies().first;
  final shows = await lib.shows().first;
  String base(String p) => p.split('/').where((s) => s.isNotEmpty).last;
  final rows = <String>[
    'kind\ttitle\toriginal\tyear\ttmdb\tconf\tposter\tfile',
  ];
  for (final m in [...movies, ...shows]) {
    rows.add(
      [
        m.kind.name,
        m.title,
        m.originalTitle ?? '',
        m.year ?? '',
        m.tmdbId ?? '',
        m.matchConfidence?.toStringAsFixed(2) ?? '',
        m.posterUrl == null ? 'no' : 'yes',
        base(m.filePath),
      ].join('\t'),
    );
  }
  var review = 0;
  final inbox = t.services.reviewInbox;
  if (inbox != null) {
    final entries = await inbox.entries();
    review = entries.length;
    rows.add('');
    rows.add('review\tstate\ttitle\tlocal\tyear\tconf\ttier\tsim\ttop');
    for (final e in entries) {
      final c = e.candidates.isEmpty ? null : e.candidates.first;
      rows.add(
        [
          e.kind,
          e.state.name,
          e.title,
          e.localTitle,
          e.year ?? '',
          e.confidence?.toStringAsFixed(2) ?? '',
          e.signals?.tier ?? '',
          e.signals?.title.toStringAsFixed(2) ?? '',
          c == null ? '' : '${c.title} (${c.year ?? '?'}) ${c.extKind}',
        ].join('\t'),
      );
    }
  }
  File('$_out/recognition-$tag.tsv').writeAsStringSync('${rows.join('\n')}\n');
  final mm = movies.where((m) => m.tmdbId != null).length;
  final sm = shows.where((m) => m.tmdbId != null).length;
  return 'movies matched $mm/${movies.length}, shows matched '
      '$sm/${shows.length}, in review $review';
}

Future<void> _pop(_Driver t) async {
  final tester = t.tester;
  for (var k = 0; k < 3; k++) {
    final back = find.byIcon(Icons.arrow_back);
    if (back.evaluate().isNotEmpty) {
      await t.tap(back);
      await t.idle(500);
      return;
    }
    final at = tester.element(find.byType(Scaffold).last);
    if (await Navigator.of(at).maybePop()) {
      await t.idle(500);
      return;
    }
  }
}

Future<void> _runExplore(_Driver t) async {
  final tester = t.tester;

  await t.step('explore: recognition state', () async {
    return _dumpRecognition(t, 'late');
  });

  for (final s in [
    Section.home,
    Section.movies,
    Section.shows,
    Section.twitch,
    Section.music,
    Section.audiobooks,
    Section.personal,
  ]) {
    await t.step('explore: section ${s.name}', () async {
      if (find.byIcon(s.icon).evaluate().isEmpty &&
          find.byIcon(s.selectedIcon).evaluate().isEmpty) {
        return 'no entry in the navigation';
      }
      await t.goTo(s);
      await t.idle(2500);
      await t.shot('x-${s.name}');
      _dumpTexts('section-${s.name}');
      // The rest of the page.
      final list = find.byType(Scrollable);
      if (list.evaluate().isNotEmpty) {
        await tester.drag(
          list.first,
          const Offset(0, -700),
          warnIfMissed: false,
        );
        await t.idle(1500);
        await t.shot('x-${s.name}-down');
        _dumpTexts('section-${s.name}-down');
      }
      return 'shown';
    });
  }

  await t.step('explore: every film and show page', () async {
    final movies = await t.services.library.movies().first;
    final shows = await t.services.library.shows().first;
    var n = 0;
    for (final (sec, items) in [
      (Section.movies, movies),
      (Section.shows, shows),
    ]) {
      for (final m in items.take(30)) {
        await t.goTo(sec);
        await t.idle(800);
        final card = find.text(m.title);
        if (!await t.scrollTo(card)) {
          _log('NOTE explore: no card for «${m.title}» in ${sec.name}');
          continue;
        }
        await t.tap(card.first);
        await t.idle(2500);
        n++;
        final name = 'page-${sec.name}-${m.id}';
        await t.shot(name);
        _dumpTexts(name);
      }
    }
    return '$n pages';
  });

  await t.step('explore: download a film', () async {
    final movies = await t.services.library.movies().first;
    final film = movies.firstWhere((m) => _isFilm(m, 'Elephants Dream'));
    await t.goTo(Section.movies);
    await t.idle(800);
    final card = find.text(film.title);
    if (!await t.scrollTo(card)) return 'no card for «${film.title}»';
    await t.tap(card.first);
    await t.idle(2000);
    final button = find.byKey(const ValueKey('offline-button'));
    if (button.evaluate().isEmpty) return 'no download button';
    await t.tap(button);
    await t.idle(2000);
    await t.shot('x-download-tapped');
    _dumpTexts('download-tapped');
    // A version picker or a sheet may ask first: take the first offer.
    final version = find.byWidgetPredicate(
      (w) =>
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith('offline-version-') &&
          (w.key! as ValueKey<String>).value != 'offline-version-dialog',
    );
    if (version.evaluate().isNotEmpty) {
      await t.tap(version.first);
      await t.idle(1500);
    }
    await t.idle(30000);
    await t.shot('x-download-30s');
    _dumpTexts('download-30s');
    await t.goTo(Section.settings);
    final open = find.byKey(const ValueKey('settings-downloads-open'));
    if (await t.scrollTo(open)) {
      await t.tap(open);
      await t.idle(3000);
      await t.shot('x-downloads-page');
      _dumpTexts('downloads-page');
      await _pop(t);
    }
    return 'tapped Скачать on «${film.title}»';
  });

  await t.step('explore: keyboard only (remote)', () async {
    await t.goTo(Section.home);
    await t.idle(1500);
    final log = <String>[];
    String focused() {
      final f = FocusManager.instance.primaryFocus;
      final ctx = f?.context;
      if (ctx == null) return 'none';
      final texts = <String>[];
      void visit(Element e) {
        final w = e.widget;
        if (w is RichText) texts.add(w.text.toPlainText());
        if (texts.length < 2) e.visitChildren(visit);
      }

      (ctx as Element).visitChildren(visit);
      return texts.isEmpty ? f!.debugLabel ?? '?' : texts.join(' / ');
    }

    Future<void> press(LogicalKeyboardKey k) async {
      try {
        await tester.sendKeyEvent(k);
      } catch (e) {
        log.add('${k.keyLabel}: not sent (${_oneLine(e)})');
      }
      await t.idle(350);
    }

    // Into the content, then along the rows, then Enter.
    for (final k in [
      LogicalKeyboardKey.arrowRight,
      LogicalKeyboardKey.arrowDown,
      LogicalKeyboardKey.arrowDown,
      LogicalKeyboardKey.arrowDown,
      LogicalKeyboardKey.arrowRight,
    ]) {
      await press(k);
      log.add('${k.keyLabel} → ${focused()}');
    }
    await t.shot('x-keys-home');
    await press(LogicalKeyboardKey.enter);
    await t.idle(2000);
    final page =
        find.byType(MovieDetailScreen).evaluate().isNotEmpty ||
        find.byType(ShowDetailScreen).evaluate().isNotEmpty;
    log.add(
      'Enter → ${page ? 'detail page' : 'no detail page'}; focus ${focused()}',
    );
    await t.shot('x-keys-enter');
    if (page) {
      await press(LogicalKeyboardKey.enter);
      final playing = await t.waitUntil(
        () => find.byType(PlayerScreen).evaluate().isNotEmpty,
        secs: 10,
      );
      log.add('Enter on the page → ${playing ? 'player' : 'no player'}');
      await t.shot('x-keys-play');
      await t.closePlayer();
    }
    return log.join('; ');
  });

  await t.step('explore: player OSD buttons', () async {
    final movies = await t.services.library.movies().first;
    final film = movies.firstWhere((m) => _isFilm(m, 'Sintel'));
    await t.goTo(Section.movies);
    await t.idle(800);
    final card = find.text(film.title);
    if (!await t.scrollTo(card)) return 'no card for «${film.title}»';
    await t.tap(card.first);
    await t.idle(1500);
    await t.tap(find.byKey(detailPlayKey));
    await t.player();
    final out = <String>[];
    for (final k in [
      'osd-audio',
      'osd-subtitles',
      'osd-speed',
      'osd-video',
      'osd-info',
      'osd-chapters',
      'osd-pip',
    ]) {
      // Wake the OSD: a mouse move over the video.
      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: const Offset(800, 500));
      await gesture.moveTo(const Offset(810, 510));
      await t.idle(600);
      final f = find.byKey(ValueKey(k));
      if (f.evaluate().isEmpty) {
        out.add('$k: absent');
        await gesture.removePointer();
        continue;
      }
      final before = find.byType(RichText).evaluate().length;
      await tester.tap(f.first, warnIfMissed: false);
      await t.idle(1500);
      final after = find.byType(RichText).evaluate().length;
      await t.shot('x-$k');
      _dumpTexts(k);
      out.add('$k: texts $before → $after');
      await t.key(LogicalKeyboardKey.escape);
      await t.idle(600);
      await gesture.removePointer();
      if (find.byType(PlayerScreen).evaluate().isEmpty) {
        out.add('player closed by Esc after $k');
        break;
      }
    }
    await t.closePlayer();
    return out.join('; ');
  });

  await t.step('explore: accept the suggestion of «Агент 327»', () async {
    final movies = await t.services.library.movies().first;
    final film = movies.firstWhere((m) => m.title.contains('327'));
    await t.goTo(Section.movies);
    await t.idle(800);
    final card = find.text(film.title);
    if (!await t.scrollTo(card)) return 'no card for «${film.title}»';
    await t.tap(card.first);
    await t.idle(2000);
    final cand = find.byWidgetPredicate(
      (w) =>
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith(
            'review-strip-candidate-',
          ),
    );
    if (cand.evaluate().isEmpty) return 'no suggestion on the page';
    await t.tap(cand.first);
    await t.idle(5000);
    await t.shot('x-agent-picked');
    _dumpTexts('agent-picked');
    final after = (await t.services.library.movies().first).firstWhere(
      (m) => m.id == film.id,
      orElse: () => film,
    );
    await t.goTo(Section.home);
    await t.idle(2000);
    await t.shot('x-home-after-pick');
    _dumpTexts('home-after-pick');
    final banner = find.byKey(const ValueKey('home-review-banner'));
    final line = banner.evaluate().isEmpty
        ? 'no banner'
        : find
              .descendant(of: banner, matching: find.byType(RichText))
              .evaluate()
              .map((e) => (e.widget as RichText).text.toPlainText())
              .join(' ');
    final inbox = await t.services.reviewInbox?.entries();
    return '«${after.title}» tmdb ${after.tmdbId}; home «$line»; '
        'inbox ${inbox?.length}';
  });

  await t.step('explore: «Состояние и журнал»', () async {
    await t.goTo(Section.settings);
    await t.idle(800);
    final f = find.text(t.l10n.statusSettingsTile);
    if (!await t.scrollTo(f)) return 'no row';
    await t.tap(f);
    await t.idle(3000);
    await t.shot('x-status');
    _dumpTexts('status');
    final list = find.byType(Scrollable);
    if (list.evaluate().isNotEmpty) {
      await tester.drag(list.last, const Offset(0, -700), warnIfMissed: false);
      await t.idle(1000);
      await t.shot('x-status-down');
      _dumpTexts('status-down');
    }
    await _pop(t);
    return 'opened';
  });

  await t.step('explore: settings top to bottom', () async {
    await t.goTo(Section.settings);
    await t.idle(1200);
    final list = find.byType(Scrollable);
    for (var i = 0; i < 14; i++) {
      await t.shot('x-settings-$i');
      _dumpTexts('settings-$i');
      await tester.drag(list.first, const Offset(0, -650), warnIfMissed: false);
      await t.idle(700);
    }
    return 'scrolled';
  });

  // Rows of settings that open a page or a dialog.
  for (final key in [
    'settings-health',
    'settings-review-inbox',
    'settings-recognition-status',
    'settings-profiles-manage',
    'settings-downloads-open',
    'settings-image-cache-usage',
    'settings-languages-sound',
    'settings-ssh-rsa-keys',
    'settings-secret-storage',
    'settings-twitch-archive',
    'settings-extra-providers',
  ]) {
    await t.step('explore: $key', () async {
      await t.goTo(Section.settings);
      await t.idle(800);
      final f = find.byKey(ValueKey(key));
      if (!await t.scrollTo(f)) return 'not on the page';
      final before = find.byType(Scaffold).evaluate().length;
      final dialogsBefore = find.byType(Dialog).evaluate().length;
      await t.tap(f);
      await t.idle(4000);
      await t.shot('x-$key');
      _dumpTexts(key);
      final opened =
          find.byType(Scaffold).evaluate().length != before ||
          find.byType(Dialog).evaluate().length != dialogsBefore;
      if (key == 'settings-health') {
        final check = find.byKey(const ValueKey('health-check'));
        if (check.evaluate().isNotEmpty) {
          await t.tap(check);
          await t.idle(8000);
          await t.shot('x-health-after-check');
          _dumpTexts('health-after-check');
        }
      }
      await _pop(t);
      return opened ? 'opened' : 'no new page or dialog';
    });
  }

  await t.step('explore: anime database «Проверить сейчас»', () async {
    await t.goTo(Section.settings);
    await t.idle(800);
    final check = find.byKey(const ValueKey('settings-anime-db-check'));
    if (!await t.scrollTo(check)) return 'no check button';
    final status = find.byKey(const ValueKey('settings-anime-db-status'));
    String text(Finder f) => f.evaluate().isEmpty
        ? ''
        : find
              .descendant(of: f, matching: find.byType(RichText))
              .evaluate()
              .map((e) => (e.widget as RichText).text.toPlainText())
              .join(' | ');
    final before = text(status);
    await t.tap(check);
    final snack = find.byType(SnackBar);
    await t.waitUntil(() => snack.evaluate().isNotEmpty, secs: 120);
    await t.idle(1000);
    await t.shot('x-anime-db-check');
    _dumpTexts('anime-db-check');
    return 'before «$before»; snack «${text(snack)}»; after «${text(status)}»';
  });

  await t.step('explore: turn Twitch on', () async {
    await t.goTo(Section.settings);
    await t.idle(800);
    final sw = find.byKey(const ValueKey('settings-show-twitch'));
    if (!await t.scrollTo(sw)) return 'no switch';
    await t.tap(sw);
    await t.idle(1500);
    await t.shot('x-twitch-switch');
    return 'switched';
  });

  await t.step('explore: sources, wrong password', () async {
    await t.goTo(Section.settings);
    final ctx = tester.element(find.byType(Scaffold).last);
    unawaited(
      Navigator.of(ctx)
          .push(MaterialPageRoute<void>(builder: (_) => const SourcesScreen())),
    );
    await t.idle(5000);
    await t.shot('x-sources');
    _dumpTexts('sources');
    await t.tap(find.text(t.l10n.addSourceTitle));
    await t.idle(1000);
    await t.shot('x-add-source');
    _dumpTexts('add-source');
    // The dialog's row, not the source card's subtitle behind it.
    await t.tap(find.text(t.l10n.sourcesTypeSmb).last);
    await t.idle(1500);
    final server = find.byKey(const ValueKey('smb-server'));
    if (server.evaluate().isNotEmpty) {
      await t.type(server, _host);
      await t.tap(find.byKey(const ValueKey('smb-next')));
    }
    await t.waitFor(find.byKey(const ValueKey('smb-user')), 'sign-in form');
    await t.type(find.byKey(const ValueKey('smb-user')), 'nobody');
    await t.type(find.byKey(const ValueKey('smb-password')), 'wrong-pass');
    await t.tap(find.byKey(const ValueKey('smb-connect')));
    final err = find.byKey(const ValueKey('smb-error'));
    final shown = await t.waitUntil(() => err.evaluate().isNotEmpty, secs: 30);
    await t.shot('x-smb-wrong-password');
    _dumpTexts('smb-wrong-password');
    final details = find.byKey(const ValueKey('smb-error-details'));
    if (details.evaluate().isNotEmpty) {
      await t.tap(details);
      await t.idle(800);
      await t.shot('x-smb-wrong-password-details');
      _dumpTexts('smb-wrong-password-details');
    }
    // An unknown host.
    for (var k = 0; k < 4; k++) {
      await t.key(LogicalKeyboardKey.escape);
      await t.idle(400);
    }
    return shown ? 'error shown' : 'no smb-error in 30 s';
  });

  await t.step('explore: search edge cases', () async {
    final out = <String>[];
    for (final q in [
      'синтел',
      'Синтел',
      'sin',
      'zzqxw',
      '   ',
      "'\"%_*",
      'Pioneer',
      '2008',
      'bunny',
      'Red',
    ]) {
      await t.goTo(Section.search);
      await t.idle(600);
      final field = find.byType(TextField).first;
      await t.type(field, q);
      await t.idle(2500);
      final name = 'search-${out.length}';
      await t.shot('x-$name');
      _dumpTexts(name);
      List<Object> core;
      try {
        core = await t.services.library.search(q);
      } catch (e) {
        core = ['error: ${_oneLine(e)}'];
      }
      out.add('«$q»: core ${core.length}');
    }
    return out.join('; ');
  });

  await t.step('explore: twitch without a login', () async {
    if (find.byIcon(Section.twitch.icon).evaluate().isEmpty) {
      return 'no Twitch section';
    }
    await t.goTo(Section.twitch);
    await t.idle(3000);
    await t.shot('x-twitch');
    _dumpTexts('twitch');
    final signIn = find.byKey(const ValueKey('twitch-sign-in-button'));
    if (signIn.evaluate().isNotEmpty) {
      await t.tap(signIn);
      await t.idle(5000);
      await t.shot('x-twitch-sign-in');
      _dumpTexts('twitch-sign-in');
      await t.key(LogicalKeyboardKey.escape);
      await t.idle(600);
    }
    final archive = find.byKey(const ValueKey('twitch-archive-open'));
    if (archive.evaluate().isNotEmpty) {
      await t.tap(archive);
      await t.idle(3000);
      await t.shot('x-twitch-archive');
      _dumpTexts('twitch-archive');
      await _pop(t);
    }
    return 'shown';
  });
}
