// SPDX-License-Identifier: MPL-2.0
//
// TWI-040: «Клип» on the live players. The first press asks for
// `clips:edit` with a new sign-in, then the «Клип готов» card offers
// «Смотреть», «Скопировать ссылку», «Поделиться» (phones) and «Открыть на
// Twitch»; a channel with clips off keeps the button off with the reason;
// 401 without the scope signs in again; 429 says how long to wait; Alt+X
// on a computer. FakeTwitchBackend stands in for the core (its recorded
// Helix answers are tested in `pa-twitch`, tests/clips.rs). Both flavors:
//   flutter test test/twitch_clip_test.dart --dart-define=KINOLUNE_FLAVOR=full
//   flutter test test/twitch_clip_test.dart --dart-define=KINOLUNE_FLAVOR=store
// Screenshots with KINOLUNE_SCREENSHOTS_DIR set.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_clip.dart';
import 'package:kinolune/screens/twitch/twitch_embed_desktop.dart'
    show twitchEmbedCovered;
import 'package:kinolune/screens/twitch/twitch_embed_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_embed.dart' show TwitchEmbedEngine;
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:kinolune/widgets/qr_code.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _desktop = Size(1280, 800);
const _phone = Size(390, 844);
const _tv = Size(1920, 1080);

class _Surface implements TwitchEmbedSurface {
  @override
  Widget build(BuildContext context) =>
      const SizedBox.expand(key: ValueKey('fake-embed-webview'));

  @override
  Future<void> runJavaScript(String script) async {}

  @override
  void dispose() {}
}

TwitchEmbedSurface _surface({
  required String html,
  required Uri baseUrl,
  required void Function(String message) onMessage,
  required void Function(Uri url) onExternalLink,
  void Function(bool fullscreen)? onFullscreen,
  VoidCallback? onBack,
}) => _Surface();

const _clip = TwitchNewClip(
  slug: 'MadeUpClipSlug',
  url: 'https://clips.twitch.tv/MadeUpClipSlug',
  editUrl: 'https://clips.twitch.tv/MadeUpClipSlug/edit',
  title: 'PixelHeron live',
  broadcasterName: 'PixelHeron',
  duration: Duration(seconds: 30),
);

/// The app with the embed player of [target] over it.
Future<void> _open(
  WidgetTester t,
  AppServices services,
  TwitchPlayTarget target, {
  FormFactor formFactor = FormFactor.desktop,
}) async {
  await t.pumpWidget(
    testApp(services, section: Section.twitch, formFactor: formFactor),
  );
  await settle(t);
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // ignore: unawaited_futures
  nav.push(
    MaterialPageRoute<void>(
      builder: (_) => TwitchEmbedScreen(
        target: target,
        surface: _surface,
        engine: TwitchEmbedEngine.webKitGtk,
      ),
    ),
  );
  await settle(t);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

/// The sign-in dialog confirms on its next poll (every 5 s).
Future<void> _confirmSignIn(WidgetTester t) async {
  expect(find.byKey(const ValueKey('twitch-sign-in')), findsOneWidget);
  await t.pump(const Duration(seconds: 6));
  await settle(t, 6);
}

AppLocalizations _l(WidgetTester t) =>
    t.element(find.byType(Scaffold).first).l10n;

FakeTwitchBackend _backend({bool signedIn = true}) =>
    FakeTwitchBackend.demo(now: demoNow, signedIn: signedIn);

void main() {
  setUpAll(loadAppFonts);
  // Dialogs a test leaves open never return: start each test uncovered.
  setUp(() => twitchEmbedCovered.value = 0);

  testWidgets('first press asks for clips:edit, then «Клип готов»', (t) async {
    setSurface(t, _desktop);
    final backend = _backend();
    final services = await twitchServices(twitch: backend);
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    final button = find.byKey(const ValueKey('twitch-clip'));
    expect(button, findsOneWidget);
    // On a computer the tooltip names Alt+X.
    expect(find.byTooltip(_l(t).twitchClipShortcut), findsOneWidget);

    await t.tap(button);
    await settle(t, 4);
    expect(
      find.byKey(const ValueKey('twitch-sign-in-clips-note')),
      findsOneWidget,
    );
    expect(backend.clipRequests, isEmpty);
    await _confirmSignIn(t);
    expect(backend.clipsScope, isTrue);
    expect(backend.clipRequests, ['7001']);
    expect(find.byKey(const ValueKey('twitch-clip-ready')), findsOneWidget);
    expect(find.text(_l(t).twitchClipReady), findsOneWidget);
    // Linux and Windows: the native web view hides under the card.
    expect(twitchEmbedCovered.value, 1);
    expect(
      find.text('https://clips.twitch.tv/DemoClipFromKinolune'),
      findsOneWidget,
    );
    for (final k in ['watch', 'copy', 'edit', 'close']) {
      expect(find.byKey(ValueKey('twitch-clip-$k')), findsOneWidget);
    }
    // No share menu on a computer: the link is copied.
    expect(find.byKey(const ValueKey('twitch-clip-share')), findsNothing);
    await capture(t, 'twi040-clip-ready-desktop');

    // «Смотреть» goes to the clip like the channel's clips (TWI-036).
    await t.tap(find.byKey(const ValueKey('twitch-clip-watch')));
    await settle(t, 4);
    expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
    Navigator.of(
      t.element(find.byKey(const ValueKey('twitch-open-dialog'))),
    ).pop();
    await settle(t, 4);
    await t.tap(find.byKey(const ValueKey('twitch-clip-close')));
    await settle(t, 4);
    expect(find.byKey(const ValueKey('twitch-clip-ready')), findsNothing);
    expect(twitchEmbedCovered.value, 0);
    await _close(t);

    // The scope stays: the next press makes the clip at once.
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    await t.tap(find.byKey(const ValueKey('twitch-clip')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('twitch-sign-in')), findsNothing);
    expect(find.byKey(const ValueKey('twitch-clip-ready')), findsOneWidget);
    expect(backend.clipRequests, ['7001', '7001']);
    await _close(t);
  });

  testWidgets('signed out: the press goes to the sign-in first', (t) async {
    setSurface(t, _desktop);
    final backend = _backend(signedIn: false);
    final services = await twitchServices(twitch: backend);
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    await t.tap(find.byKey(const ValueKey('twitch-clip')));
    await settle(t, 4);
    await _confirmSignIn(t);
    expect(backend.signedIn, isTrue);
    expect(find.byKey(const ValueKey('twitch-clip-ready')), findsOneWidget);
    await _close(t);
  });

  testWidgets('401 without the scope signs in again, then the clip', (t) async {
    setSurface(t, _desktop);
    final backend = _backend()..clipsScope = true;
    backend.clipAnswers.add(
      const TwitchClipResult(TwitchClipOutcome.scopeRequired),
    );
    final services = await twitchServices(twitch: backend);
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    await t.tap(find.byKey(const ValueKey('twitch-clip')));
    await settle(t, 4);
    await _confirmSignIn(t);
    expect(backend.clipRequests, ['7001', '7001']);
    expect(find.byKey(const ValueKey('twitch-clip-ready')), findsOneWidget);
    await _close(t);
  });

  testWidgets('clips off: the button stays off with the reason', (t) async {
    setSurface(t, _desktop);
    final backend = _backend()..clipsScope = true;
    backend.clipsDisabled.add('7001');
    final services = await twitchServices(twitch: backend);
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    await t.tap(find.byKey(const ValueKey('twitch-clip')));
    await settle(t, 6);
    final l = _l(t);
    expect(find.text(l.twitchClipsOff), findsOneWidget);
    expect(find.byKey(const ValueKey('twitch-clip-ready')), findsNothing);
    expect(
      t
          .widget<ButtonStyleButton>(find.byKey(const ValueKey('twitch-clip')))
          .onPressed,
      isNull,
    );
    expect(find.byTooltip(l.twitchClipsOff), findsOneWidget);
    expect(services.twitch.clipsOffReason('pixelheron'), isNotNull);
    await _close(t);
  });

  testWidgets('429, offline, not ready and failure say why', (t) async {
    setSurface(t, _desktop);
    final backend = _backend()..clipsScope = true;
    backend.clipAnswers.addAll(const [
      TwitchClipResult(
        TwitchClipOutcome.rateLimited,
        retryAfter: Duration(seconds: 12),
      ),
      TwitchClipResult(TwitchClipOutcome.offline),
      TwitchClipResult(TwitchClipOutcome.notReady),
      TwitchClipResult(TwitchClipOutcome.network),
    ]);
    final services = await twitchServices(twitch: backend);
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    final l = _l(t);
    for (final text in [
      l.twitchClipRateLimited(12),
      l.twitchClipOffline,
      l.twitchClipNotReady,
      l.twitchClipFailed,
    ]) {
      await t.tap(find.byKey(const ValueKey('twitch-clip')));
      await settle(t, 6);
      expect(find.text(text), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-clip-ready')), findsNothing);
      ScaffoldMessenger.of(t.element(find.byType(Scaffold).last))
          .hideCurrentSnackBar();
      await settle(t, 6);
    }
    await _close(t);
  });

  testWidgets('Alt+X makes a clip on a computer; VODs have no button', (
    t,
  ) async {
    setSurface(t, _desktop);
    final backend = _backend()..clipsScope = true;
    final services = await twitchServices(twitch: backend);
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    await t.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.keyX);
    await t.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await settle(t, 6);
    expect(backend.clipRequests, ['7001']);
    expect(find.byKey(const ValueKey('twitch-clip-ready')), findsOneWidget);
    await _close(t);

    await _open(
      t,
      services,
      const TwitchPlayTarget.vod('9101', login: 'pixelheron'),
    );
    expect(find.byKey(const ValueKey('twitch-clip')), findsNothing);
    await _close(t);
  });

  testWidgets('without a Twitch client id there is no button', (t) async {
    setSurface(t, _desktop);
    final services = await twitchServices(
      twitch: FakeTwitchBackend.demo(now: demoNow, canSignIn: false),
    );
    await _open(t, services, const TwitchPlayTarget.live('pixelheron'));
    expect(find.byKey(const ValueKey('twitch-clip')), findsNothing);
    await _close(t);
  });

  group('«Клип готов» card', () {
    Future<void> show(WidgetTester t, FormFactor f) async {
      final services = await twitchServices();
      await t.pumpWidget(
        testApp(services, section: Section.twitch, formFactor: f),
      );
      await settle(t);
      // ignore: unawaited_futures
      showTwitchClipReady(t.element(find.byType(Scaffold).first), _clip);
      await settle(t, 4);
      expect(find.byKey(const ValueKey('twitch-clip-ready')), findsOneWidget);
      expect(find.text('PixelHeron live'), findsOneWidget);
      expect(find.text('PixelHeron · 0:30'), findsOneWidget);
      expect(find.text(_clip.url), findsOneWidget);
    }

    testWidgets('phone: «Поделиться» opens the system menu', (t) async {
      setSurface(t, _phone, dpr: 3);
      final shared = <Object?>[];
      t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        const MethodChannel('app.kinolune/share'),
        (call) async {
          if (call.method == 'shareText') shared.add(call.arguments);
          return true;
        },
      );
      addTearDown(
        () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          const MethodChannel('app.kinolune/share'),
          null,
        ),
      );
      await show(t, FormFactor.phone);
      await capture(t, 'twi040-clip-ready-phone');
      await t.tap(find.byKey(const ValueKey('twitch-clip-share')));
      await settle(t, 2);
      expect(shared, [_clip.url]);
      expect(find.byType(QrCodeView), findsNothing);
      await _close(t);
    });

    testWidgets('computer: copy and open, no share', (t) async {
      setSurface(t, _desktop);
      await show(t, FormFactor.desktop);
      expect(find.byKey(const ValueKey('twitch-clip-share')), findsNothing);
      expect(find.byKey(const ValueKey('twitch-clip-copy')), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-clip-edit')), findsOneWidget);
      await _close(t);
    });

    testWidgets('TV: QR code, «Смотреть» focused, edit link as QR', (t) async {
      setSurface(t, _tv);
      await show(t, FormFactor.tv);
      expect(find.byType(QrCodeView), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-clip-share')), findsNothing);
      final watch = find.byKey(const ValueKey('twitch-clip-watch'));
      expect(t.widget<ButtonStyleButton>(watch).autofocus, isTrue);
      await capture(t, 'twi040-clip-ready-tv');
      // A TV opens nothing: «Открыть на Twitch» shows the edit link.
      await t.tap(find.byKey(const ValueKey('twitch-clip-edit')));
      await settle(t, 4);
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
      expect(find.text(_clip.editUrl), findsOneWidget);
      await _close(t);
    });
  });
}
