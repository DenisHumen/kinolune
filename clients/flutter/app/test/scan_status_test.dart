// SPDX-License-Identifier: MPL-2.0
//
// LIB-023: the scan progress names the step with its counts and the
// provider that makes the scan wait; a step without a count animates the
// bar instead of freezing it. With KINOLUNE_SCREENSHOTS_DIR set the frames
// are written for docs/screenshots/progress/lib-023:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/lib-023 \
//     flutter test test/scan_status_test.dart

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/scan_status.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

/// A demo library whose scan reports [states] and then waits.
class _HeldScan extends FakeLibrary {
  _HeldScan() : super.demo(scanStepDelay: Duration.zero);

  final _states = StreamController<ScanProgress>();
  final _release = Completer<void>();

  void report(ScanProgress p) => _states.add(p);

  Future<void> finish() async {
    if (!_release.isCompleted) _release.complete();
    await _states.close();
  }

  @override
  Stream<ScanProgress> scan({String? sourceId}) async* {
    await for (final p in _states.stream) {
      yield p;
    }
    await _release.future;
    yield const ScanProgress(phase: ScanPhase.done, filesFound: 1971);
  }
}

const _probing = ScanProgress(
  phase: ScanPhase.metadata,
  step: ScanStep.probing,
  filesFound: 1971,
  processed: 87,
  total: 236,
);

const _titles = ScanProgress(
  phase: ScanPhase.metadata,
  step: ScanStep.titles,
  filesFound: 1971,
  processed: 120,
  total: 200,
);

const _animeWaiting = ScanProgress(
  phase: ScanPhase.metadata,
  step: ScanStep.anime,
  filesFound: 1971,
  processed: 31,
  total: 70,
  waiting: [
    ProviderWait(provider: 'anilist', seconds: 40, providerAsked: true),
  ],
);

Future<AppLocalizations> _ru() =>
    AppLocalizations.delegate.load(const Locale('ru'));

void main() {
  setUpAll(loadAppFonts);

  test('the line names the step, its counts and the provider wait', () async {
    final l = await _ru();
    expect(
      scanStatusText(l, _titles),
      'Распознано 120 из 200 · загружаем названия и постеры',
    );
    expect(scanStatusText(l, _probing), 'Читаем заголовки файлов… 87 из 236');
    expect(
      scanStatusText(l, _animeWaiting),
      'Уточняем аниме (AniList, Shikimori)… 31 из 70 · '
      'AniList просит подождать 40 с',
    );
    const pacing = ScanProgress(
      phase: ScanPhase.metadata,
      step: ScanStep.titles,
      processed: 1,
      total: 2,
      waiting: [
        ProviderWait(provider: 'tmdb', seconds: 2),
        ProviderWait(provider: 'shikimori', seconds: 5),
      ],
    );
    expect(scanWaitText(l, pacing), 'Ждём Shikimori: лимит запросов, 5 с');
    // Listing has no total: the bar animates, the line counts files.
    const listing = ScanProgress(
      phase: ScanPhase.scanning,
      step: ScanStep.listing,
      filesFound: 342,
    );
    expect(scanFraction(listing), isNull);
    expect(scanStatusText(l, listing), 'Ищем файлы… найдено 342');
    // Older libraries report only the phase.
    expect(
      scanStatusText(l, const ScanProgress(phase: ScanPhase.connecting)),
      l.settingsScanConnecting,
    );
    expect(scanFraction(_titles), closeTo(0.6, 1e-9));
  });

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('progress of a large scan on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final library = _HeldScan();
      final services = await demoServices(library: library);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(
          services,
          formFactor: ff,
          section: name == 'desktop' ? Section.home : Section.settings,
        ),
      );
      await settle(t);
      unawaited(services.scan());
      final shots = name == 'desktop'
          ? [('01', _probing), ('02', _titles), ('03', _animeWaiting)]
          : [('04', _animeWaiting)];
      for (final (n, p) in shots) {
        library.report(p);
        await settle(t, 4);
        expect(services.scanning, isTrue);
        final l = await _ru();
        expect(find.textContaining(scanStepText(l, p)), findsWidgets);
        if (p.waiting.isNotEmpty) {
          expect(find.textContaining('AniList просит подождать'), findsWidgets);
        }
        await capture(t, '$n-scan-${p.step!.name}-$name');
      }
      await library.finish();
      await settle(t, 4);
      expect(services.scanning, isFalse);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
