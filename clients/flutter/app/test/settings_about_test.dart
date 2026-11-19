// SPDX-License-Identifier: MPL-2.0
//
// UI-016: Settings sections and their stored values, About (version, build
// flavor and capabilities, licences, attributions) and the donation links,
// which only full builds show. Run in both flavors:
//   flutter test test/settings_about_test.dart
//   flutter test test/settings_about_test.dart --dart-define=KINOLUNE_FLAVOR=full

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/branding.dart';
import 'package:kinolune/build_info.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/integration/library.dart'
    show RecognitionStatus, ScanPhase, ScanProgress, ScanStep;
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/licenses.dart';
import 'package:kinolune/screens/about_screen.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show ReplayGainMode;
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _en = lookupAppLocalizations(const Locale('en'));

/// Fictional accounts: the real ones are the owner's to fill in.
const _links = [
  DonationLink('Example Fund', 'https://donate.example.org/kinolune'),
  DonationLink('Not set up', ''),
];

Future<AppServices> _services({
  MemoryPreferences? prefs,
  ImageCacheControl? cache,
  BuildInfo buildInfo = const BuildInfo(),
}) async {
  final s = AppServices(
    library: FakeLibrary.demo(scanStepDelay: Duration.zero),
    secureStore: MemorySecureStore(),
    preferences: prefs,
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => demoNow,
    imageCache: cache,
    buildInfo: buildInfo,
  );
  await s.init();
  // UI-041: these tests cover the advanced sections too.
  await s.setShowAdvancedSettings(true);
  return s;
}

Future<void> _openSettings(WidgetTester t, AppServices s) async {
  setSurface(t, const Size(1280, 800));
  await t.pumpWidget(testApp(s, section: Section.settings));
  await settle(t, 8);
}

/// The page's list (not the scrollables of text fields inside it).
Finder _page() {
  final about = find.byType(AboutScreen);
  final screen = about.evaluate().isNotEmpty
      ? about
      : find.byType(SettingsScreen);
  return find.descendant(of: screen, matching: find.byType(Scrollable)).first;
}

/// Scrolls [f] into view; the lists build lazily, so it may not exist yet.
Future<void> _reveal(WidgetTester t, Finder f) async {
  if (f.evaluate().isEmpty) {
    await t.scrollUntilVisible(f, 200, scrollable: _page());
  }
  await t.ensureVisible(f);
  await settle(t, 3);
}

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = find.byKey(ValueKey(key));
  await _reveal(t, f);
  await t.tap(f);
  await settle(t, 6);
}

Future<void> _openAbout(
  WidgetTester t, {
  AppServices? services,
  List<DonationLink>? links,
}) async {
  await _openSettings(t, services ?? await _services());
  Navigator.of(t.element(find.byType(SettingsScreen))).push(
    MaterialPageRoute<void>(builder: (_) => AboutScreen(donationLinks: links)),
  );
  await settle(t, 8);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

void main() {
  group('Settings', () {
    testWidgets('sections in order: basic, then advanced (UI-041)', (t) async {
      await _openSettings(t, await _services());
      const order = [
        'library',
        'language',
        'devices',
        'about',
        'playback',
        'shows',
        'music',
        'metadata',
        'network',
        'twitch',
        'storage',
        'privacy',
      ];
      double? last;
      for (final name in order) {
        final f = find.byKey(ValueKey('settings-section-$name'));
        await _reveal(t, f);
        expect(f, findsOneWidget, reason: name);
        final y =
            t.getTopLeft(f).dy +
            t.state<ScrollableState>(_page()).position.pixels;
        if (last != null) expect(y, greaterThan(last), reason: name);
        last = y;
      }
      await _close(t);
    });

    testWidgets('library: automatic check and recognition status (LIB-024)', (
      t,
    ) async {
      final s = await _services();
      final lib = s.library as FakeLibrary;
      await _openSettings(t, s);
      final check = find.byKey(const ValueKey('settings-auto-check'));
      final status = find.byKey(const ValueKey('settings-recognition-status'));
      await _reveal(t, status);
      expect(find.text(_en.settingsLibraryAutoHint), findsOneWidget);
      expect(
        t.widget<Text>(check).data,
        _en.settingsAutoCheckLast(_en.settingsAutoCheckJustNow),
      );
      expect(t.widget<Text>(status).data, _en.settingsRecognitionDone);

      lib.recognitionStatus = RecognitionStatus(
        lastCheck: DateTime.now().subtract(const Duration(minutes: 7)),
        worksWaiting: 5,
        unrecognizedFiles: 2,
        worksFailed: 1,
      );
      await settle(t, 3);
      expect(
        t.widget<Text>(check).data,
        _en.settingsAutoCheckLast(_en.settingsAutoCheckMinutesAgo(7)),
      );
      expect(t.widget<Text>(status).data, _en.settingsRecognitionWaiting(7));
      expect(find.text(_en.settingsRecognitionRetry(1)), findsOneWidget);

      lib.recognitionStatus = lib.recognitionStatus.copyWith(
        running: true,
        progress: () => const ScanProgress(
          phase: ScanPhase.metadata,
          step: ScanStep.titles,
          processed: 3,
          total: 5,
        ),
      );
      await settle(t, 3);
      expect(t.widget<Text>(status).data, _en.settingsScanStepTitles(3, 5));

      lib.recognitionStatus = const RecognitionStatus(
        lastCheckError: 'NAS: connection refused',
      );
      await settle(t, 3);
      expect(t.widget<Text>(check).data, _en.settingsAutoCheckNotYet);
      lib.recognitionStatus = RecognitionStatus(
        lastCheck: DateTime.now(),
        lastCheckError: 'NAS: connection refused',
      );
      await settle(t, 3);
      expect(
        t.widget<Text>(check).data,
        _en.settingsAutoCheckFailed('NAS: connection refused'),
      );
      // The manual button stays.
      expect(find.text(_en.settingsScanNow), findsOneWidget);
      await _close(t);
    });

    testWidgets('continuous playback is on by default and kept', (t) async {
      final prefs = MemoryPreferences();
      final s = await _services(prefs: prefs);
      expect(s.continuousPlayback.value, isTrue);
      await _openSettings(t, s);
      await _tapKey(t, 'settings-continuous-playback');
      expect(s.continuousPlayback.value, isFalse);
      expect(prefs.values[Preferences.continuousPlayback], 'false');
      final again = await _services(prefs: prefs);
      expect(again.continuousPlayback.value, isFalse);
      await _close(t);
    });

    testWidgets('skip interval of the player is picked and kept', (t) async {
      final prefs = MemoryPreferences();
      final s = await _services(prefs: prefs);
      expect(s.skipSeconds.value, 10);
      await _openSettings(t, s);
      await _reveal(t, find.byKey(const ValueKey('settings-skip-interval')));
      await _tapKey(t, 'settings-skip-interval');
      // 5, 10, 15, 30, 60 s.
      await t.tap(find.byKey(const ValueKey('settings-skip-interval-3')));
      await settle(t, 6);
      expect(s.skipSeconds.value, 30);
      expect(prefs.values[Preferences.skipSeconds], '30');
      final again = await _services(prefs: prefs);
      expect(again.skipSeconds.value, 30);
      await _close(t);
    });

    testWidgets('default episode order is picked from a dialog', (t) async {
      final prefs = MemoryPreferences();
      final s = await _services(prefs: prefs);
      await _openSettings(t, s);
      await _reveal(t, find.byKey(const ValueKey('settings-episode-order')));
      expect(find.text(_en.showOrderTmdb), findsOneWidget);
      await _tapKey(t, 'settings-episode-order');
      // TMDB, absolute, AniDB.
      await t.tap(find.byKey(const ValueKey('settings-episode-order-2')));
      await settle(t, 6);
      expect(s.defaultEpisodeOrder.value.name, 'anidb');
      expect(prefs.values[Preferences.defaultEpisodeOrder], 'anidb');
      expect(find.text(_en.showOrderAnidb), findsOneWidget);
      await _close(t);
    });

    testWidgets('music: gapless and ReplayGain go to the player', (t) async {
      final prefs = MemoryPreferences();
      final s = await _services(prefs: prefs);
      expect(s.music.audioProfile.gapless, isTrue);
      expect(s.music.audioProfile.replayGain, ReplayGainMode.auto);
      await _openSettings(t, s);
      await _tapKey(t, 'settings-music-gapless');
      expect(s.music.audioProfile.gapless, isFalse);
      expect(prefs.values[Preferences.musicGapless], 'false');
      await _tapKey(t, 'settings-replay-gain');
      // Auto, track, album, off.
      await t.tap(find.byKey(const ValueKey('settings-replay-gain-2')));
      await settle(t, 6);
      expect(s.music.audioProfile.replayGain, ReplayGainMode.album);
      expect(prefs.values[Preferences.musicReplayGain], 'album');
      final again = await _services(prefs: prefs);
      expect(again.music.audioProfile.gapless, isFalse);
      expect(again.music.audioProfile.replayGain, ReplayGainMode.album);
      await _close(t);
    });

    testWidgets('metadata language follows the UI unless chosen', (t) async {
      final prefs = MemoryPreferences();
      final s = await _services(prefs: prefs);
      final lib = s.library as FakeLibrary;
      await s.setLocale(const Locale('en'));
      expect(lib.metadataLanguage, 'en-US');
      await _openSettings(t, s);
      await _tapKey(t, 'settings-metadata-language');
      // Same as the interface, English, Russian.
      await t.tap(find.byKey(const ValueKey('settings-metadata-language-2')));
      await settle(t, 6);
      expect(lib.metadataLanguage, 'ru-RU');
      expect(prefs.values[Preferences.metadataLanguage], 'ru-RU');
      // The UI language no longer changes it.
      await s.setLocale(const Locale('en'));
      expect(lib.metadataLanguage, 'ru-RU');
      // Back to "same as the interface".
      await _tapKey(t, 'settings-metadata-language');
      await t.tap(find.byKey(const ValueKey('settings-metadata-language-0')));
      await settle(t, 6);
      expect(s.metadataLanguage.value, isNull);
      expect(lib.metadataLanguage, 'en-US');
      expect(prefs.values.containsKey(Preferences.metadataLanguage), isFalse);
      await _close(t);
    });

    testWidgets('storage: image cache usage, limit and clearing', (t) async {
      final prefs = MemoryPreferences();
      final cache = MemoryImageCacheControl(bytes: 87 << 20, files: 1240);
      final s = await _services(prefs: prefs, cache: cache);
      await _openSettings(t, s);
      await _reveal(
        t,
        find.byKey(const ValueKey('settings-image-cache-usage')),
      );
      expect(
        find.text(_en.settingsImageCacheUsage(1240, '87 MB', '256 MB')),
        findsOneWidget,
      );
      await _tapKey(t, 'settings-image-cache-limit');
      // 128 MB, 256 MB, 512 MB, 1 GB, 2 GB.
      await t.tap(find.byKey(const ValueKey('settings-image-cache-limit-0')));
      await settle(t, 6);
      expect(cache.maxBytes, 128 << 20);
      expect(prefs.values[Preferences.imageCacheMaxBytes], '${128 << 20}');
      await _tapKey(t, 'settings-image-cache-clear');
      expect(cache.bytes, 0);
      expect(find.text(_en.settingsImageCacheCleared), findsOneWidget);
      expect(
        find.text(_en.settingsImageCacheUsage(0, '0 MB', '128 MB')),
        findsOneWidget,
      );
      // The limit is applied again on the next start.
      final next = MemoryImageCacheControl();
      await _services(prefs: prefs, cache: next);
      expect(next.maxBytes, 128 << 20);
      await _close(t);
    });

    testWidgets('storage without the core says so', (t) async {
      await _openSettings(t, await _services());
      final f = find.text(_en.settingsImageCacheUnavailable);
      await _reveal(t, f);
      expect(f, findsOneWidget);
      await _close(t);
    });

    testWidgets('privacy: clearing the search history', (t) async {
      final prefs = MemoryPreferences()
        ..values[Preferences.searchHistory] = '["orbit"]';
      final s = await _services(prefs: prefs);
      await _openSettings(t, s);
      await _tapKey(t, 'settings-clear-search');
      expect(prefs.values.containsKey(Preferences.searchHistory), isFalse);
      expect(find.text(_en.settingsPrivacySearchCleared), findsOneWidget);
      await _close(t);
    });
  });

  group('About', () {
    testWidgets('version, flavor and capabilities of this build', (t) async {
      await _openAbout(t);
      expect(find.text(_en.aboutVersion(kAppVersion)), findsOneWidget);
      final flavor = AppFlavor.isFull
          ? _en.aboutFlavorFull('full')
          : _en.aboutFlavorStore('store');
      expect(find.text(flavor), findsOneWidget);
      expect(
        find.text(
          AppFlavor.isFull
              ? [
                  _en.aboutCapTwitchNative,
                  _en.aboutCapTwitchArchive,
                  _en.aboutCapDonationLinks,
                  _en.aboutCapSelfUpdate,
                ].join(', ')
              : _en.aboutCapabilitiesNone,
        ),
        findsOneWidget,
      );
      // No native core in widget tests.
      expect(find.text(_en.aboutCoreMissing), findsOneWidget);
      await _close(t);
    });

    testWidgets('core version and a flavor mismatch are shown', (t) async {
      final other = AppFlavor.isFull ? 'store' : 'full';
      final s = await _services(
        buildInfo: BuildInfo(
          coreVersion: '9.8.7',
          coreFlavor: other,
          coreCapabilities: const [],
        ),
      );
      await _openAbout(t, services: s);
      final otherName = AppFlavor.isFull
          ? _en.aboutFlavorStore(other)
          : _en.aboutFlavorFull(other);
      expect(find.text(_en.aboutCoreLine('9.8.7', otherName)), findsOneWidget);
      expect(find.text(_en.aboutFlavorMismatch(other)), findsOneWidget);
      await _close(t);
    });

    testWidgets('source code, licences, LGPL libraries and attributions', (
      t,
    ) async {
      await _openAbout(t);
      expect(find.text(_en.aboutSourceCode), findsOneWidget);
      expect(find.byKey(const ValueKey('about-repo')), findsOneWidget);
      for (final c in kNativeComponents) {
        final tile = find.byKey(ValueKey('about-native-${c.name}'));
        await _reveal(t, tile);
        expect(tile, findsOneWidget);
      }
      expect(
        kNativeComponents.map((c) => c.name),
        containsAll(['libmpv', 'FFmpeg', 'libsmb2']),
      );
      for (final heading in [
        _en.aboutTmdbHeading,
        _en.aboutMusicBrainzHeading,
        _en.aboutTwitchHeading,
      ]) {
        final f = find.text(heading);
        await _reveal(t, f);
        expect(f, findsOneWidget);
      }
      expect(find.text(_en.commonTmdbNotice), findsOneWidget);
      // Activating a link copies it.
      String? copied;
      t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'Clipboard.setData') {
            copied = (call.arguments as Map)['text'] as String?;
          }
          return null;
        },
      );
      addTearDown(
        () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      await _tapKey(t, 'about-third-party');
      expect(copied, kThirdPartyNoticesUrl);
      expect(find.text(_en.aboutLinkCopied), findsOneWidget);
      await _close(t);
    });

    // ROADMAP UI-016 done when: no donation links in the store flavor.
    testWidgets('donation links only with the donation-links capability', (
      t,
    ) async {
      await _openAbout(t, links: _links);
      final section = find.byKey(const ValueKey('about-donations'));
      if (AppFlavor.donationLinks) {
        await _reveal(t, section);
        expect(section, findsOneWidget);
        expect(find.text('Example Fund'), findsOneWidget);
        // A link without a URL (a placeholder) is not shown.
        expect(find.text('Not set up'), findsNothing);
      } else {
        expect(section, findsNothing);
        expect(find.text(_en.aboutSupportTitle), findsNothing);
        expect(find.text('Example Fund'), findsNothing);
        expect(
          find.textContaining('donate.example.org', findRichText: true),
          findsNothing,
        );
        for (final link in kDonationLinks) {
          expect(find.textContaining(link.platform), findsNothing);
        }
      }
      await _close(t);
    });

    testWidgets('the placeholders show no donation section', (t) async {
      await _openAbout(t);
      expect(kDonationLinks.every((l) => l.url.isEmpty), isTrue);
      expect(find.byKey(const ValueKey('about-donations')), findsNothing);
      await _close(t);
    });
  });

  group('licences', () {
    test(
      'libsmb2 and the third-party notices are on the licence page',
      () async {
        registerAppLicenses();
        final packages = <String>{};
        await for (final e in LicenseRegistry.licenses) {
          packages.addAll(e.packages);
        }
        expect(
          packages,
          containsAll(['libmpv', 'FFmpeg', 'libsmb2', 'Third-party notices']),
        );
      },
    );

    test('About lists what THIRD-PARTY-NOTICES.md lists', () {
      final notices = File('../../../packaging/app/THIRD-PARTY-NOTICES.md')
          .readAsStringSync();
      for (final c in kNativeComponents) {
        expect(notices, contains(c.name), reason: c.name);
        expect(notices, contains(c.recipe.split('/').last), reason: c.name);
      }
      expect(
        kThirdPartyNoticesUrl,
        endsWith('packaging/app/THIRD-PARTY-NOTICES.md'),
      );
      expect(
        File('../../../docs/legal/privacy-policy.md').existsSync(),
        isTrue,
      );
    });
  });
}
