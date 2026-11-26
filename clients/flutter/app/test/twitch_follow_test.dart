// SPDX-License-Identifier: MPL-2.0
//
// Follow and unfollow from the app (TWI-033, ADR-NNNN) over the fake
// Helix: Helix can only read follows, so a press adds the channel to the
// Kinolune favourites and opens its page on Twitch (the link opener is
// faked); coming back to the app reads `channels/followed` again and the
// button says «Вы фолловите на Twitch ✓». Unfollow removes the favourite.
// Signed out the button is the favourite only; a TV shows the QR window.

import 'package:flutter/gestures.dart' show kSecondaryButton;
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_browse.dart';
import 'package:kinolune/screens/twitch/twitch_follow.dart';
import 'package:kinolune/screens/twitch/twitch_widgets.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/external_links.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_backend.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:kinolune/widgets/focusable.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _desktop = Size(1280, 800);
const _phone = Size(390, 844);
const _tv = Size(960, 540);

/// Not a favourite of the demo and, here, not followed at first.
const _login = 'embertide';
const _id = '7006';

final _follow = find.byKey(const ValueKey('twitch-follow'));

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 10);
}

/// The Streams section with the channel page of [_login] open.
Future<(AppServices, FakeTwitchBackend, List<Uri>)> _channelPage(
  WidgetTester t, {
  Size size = _desktop,
  FormFactor ff = FormFactor.desktop,
  bool signedIn = true,
  bool opens = true,
}) async {
  setSurface(t, size);
  final helix = FakeTwitchBackend.demo(now: demoNow, signedIn: signedIn)
    ..follows = ['7001', '7002', '7003'];
  final opened = <Uri>[];
  final before = openExternalLink;
  openExternalLink = (url) async {
    opened.add(url);
    return opens;
  };
  addTearDown(() => openExternalLink = before);
  final services = await twitchServices(twitch: helix);
  await t.pumpWidget(
    testApp(services, formFactor: ff, section: Section.twitch),
  );
  await settle(t);
  final context = t.element(
    find.byKey(const ValueKey('twitch-favorite-quietkettle')),
  );
  openTwitchChannel(context, services.twitch.channelOf(_login));
  await settle(t, 16);
  expect(
    find.byKey(const ValueKey('twitch-channel-page-$_login')),
    findsOneWidget,
  );
  return (services, helix, opened);
}

Future<void> _close(WidgetTester t) async {
  TwitchFollowReturn.cancel();
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

/// The user switched to the browser and back.
Future<void> _comeBack(WidgetTester t) async {
  t.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
  await t.pump();
  t.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
  await settle(t, 6);
}

String _label(WidgetTester t) => switch (t.widget(_follow.first)) {
  KButton(:final label) => label,
  IconButton(:final tooltip) => tooltip!,
  _ => throw StateError('no follow button'),
};

AppLocalizations _l(WidgetTester t) => t.element(_follow.first).l10n;

void main() {
  setUpAll(loadAppFonts);

  test('the follow status comes from channels/followed', () async {
    final helix = FakeTwitchBackend.demo(now: demoNow)..follows = ['7001'];
    final twitch = TwitchService(
      backend: helix,
      preferences: MemoryPreferences(),
      clock: () => demoNow,
    );
    await twitch.init();
    expect(twitch.followsOnTwitch('pixelheron'), isNull);
    expect(await twitch.refreshFollowed('pixelheron'), isTrue);
    expect(twitch.followsOnTwitch(_login), isFalse);
    helix.follows = ['7001', _id];
    expect(await twitch.refreshFollowed(_login), isTrue);
    await twitch.signOut();
    expect(twitch.followsOnTwitch(_login), isNull);
    expect(await twitch.refreshFollowed(_login), isNull);
  });

  for (final (name, size, ff) in [
    ('desktop', _desktop, FormFactor.desktop),
    ('phone', _phone, FormFactor.phone),
  ]) {
    testWidgets('$name: follow, come back, unfollow', (t) async {
      final (services, helix, opened) = await _channelPage(
        t,
        size: size,
        ff: ff,
      );
      final twitch = services.twitch;
      final l = _l(t);
      expect(twitch.isFavorite(_login), isFalse);
      expect(_label(t), l.twitchFollow);
      await capture(t, 'twi033-$name-follow');

      // Press: a Kinolune favourite at once, the channel page on Twitch.
      await _tap(t, _follow);
      expect(twitch.isFavorite(_login), isTrue);
      expect(opened, [Uri.parse('https://www.twitch.tv/$_login')]);
      expect(TwitchFollowReturn.pending, isTrue);
      expect(_label(t), l.twitchFollow, reason: 'not followed on Twitch yet');

      // The user pressed Follow on Twitch and came back.
      helix.follows = [...helix.follows, _id];
      helix.calls.clear();
      await _comeBack(t);
      expect(helix.calls, contains('followed'));
      expect(TwitchFollowReturn.pending, isFalse);
      expect(_label(t), l.twitchFollowingOnTwitch);
      await capture(t, 'twi033-$name-following');

      // Unfollow: confirmed, the favourite goes, Twitch opens again.
      await _tap(t, _follow);
      expect(
        find.byKey(const ValueKey('twitch-unfollow-dialog')),
        findsOneWidget,
      );
      await _tap(t, find.byKey(const ValueKey('twitch-unfollow-confirm')));
      expect(twitch.isFavorite(_login), isFalse);
      expect(opened, hasLength(2));
      helix.follows = [
        for (final id in helix.follows)
          if (id != _id) id,
      ];
      await _comeBack(t);
      expect(_label(t), l.twitchFollow);
      await _close(t);
    });
  }

  testWidgets('cancelling the unfollow keeps everything', (t) async {
    final (services, helix, opened) = await _channelPage(t);
    helix.follows = [...helix.follows, _id];
    await services.twitch.addFavorite(services.twitch.channelOf(_login));
    await services.twitch.refreshFollowed();
    await settle(t, 4);
    await _tap(t, _follow);
    await _tap(t, find.text(_l(t).commonCancel));
    expect(services.twitch.isFavorite(_login), isTrue);
    expect(opened, isEmpty);
    await _close(t);
  });

  testWidgets('signed out the button is the Kinolune favourite only', (
    t,
  ) async {
    final (services, _, opened) = await _channelPage(t, signedIn: false);
    expect(_follow, findsNothing);
    final star = find.byKey(const ValueKey('twitch-channel-favorite'));
    final l = t.element(star).l10n;
    expect(t.widget<KButton>(star).label, l.twitchAddFavorite);
    await _tap(t, star);
    expect(services.twitch.isFavorite(_login), isTrue);
    expect(t.widget<KButton>(star).label, l.twitchInKinoluneFavorites);
    expect(opened, isEmpty);
    await _close(t);
  });

  testWidgets('TV: the QR window, closing it reads the status again', (
    t,
  ) async {
    final (services, helix, opened) = await _channelPage(
      t,
      size: _tv,
      ff: FormFactor.tv,
    );
    final l = _l(t);
    await _tap(t, _follow);
    expect(services.twitch.isFavorite(_login), isTrue);
    expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
    expect(find.text('https://www.twitch.tv/$_login'), findsOneWidget);
    expect(find.text(l.twitchFollowQrHint), findsOneWidget);
    expect(find.byKey(const ValueKey('twitch-open-link')), findsNothing);
    expect(opened, isEmpty);
    await capture(t, 'twi033-tv-qr');
    helix.follows = [...helix.follows, _id];
    await _tap(t, find.text(l.twitchClose));
    expect(_label(t), l.twitchFollowingOnTwitch);
    await _close(t);
  });

  testWidgets('a link nothing opens falls back to the QR window', (t) async {
    final (_, _, opened) = await _channelPage(t, opens: false);
    await _tap(t, _follow);
    expect(opened, hasLength(1));
    expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
    await _close(t);
  });

  testWidgets('the live card menu follows on Twitch', (t) async {
    setSurface(t, _desktop);
    final helix = FakeTwitchBackend.demo(now: demoNow)..follows = [];
    final opened = <Uri>[];
    final before = openExternalLink;
    openExternalLink = (url) async {
      opened.add(url);
      return true;
    };
    addTearDown(() => openExternalLink = before);
    final services = await twitchServices(twitch: helix);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    final card = find.byType(TwitchLiveCard).first;
    final login = t.widget<TwitchLiveCard>(card).stream.login;
    await t.ensureVisible(card);
    await settle(t, 4);
    await t.tap(card, buttons: kSecondaryButton);
    await settle(t, 6);
    final item = find.byKey(const ValueKey('twitch-stream-menu-follow'));
    expect(item, findsOneWidget);
    await t.tap(item);
    await settle(t, 10);
    expect(opened, [Uri.parse('https://www.twitch.tv/$login')]);
    expect(services.twitch.isFavorite(login), isTrue);
    TwitchFollowReturn.cancel();
    await _close(t);
  });

  testWidgets('signed out, the live card menu has no follow', (t) async {
    setSurface(t, _desktop);
    final services = await twitchServices(
      twitch: FakeTwitchBackend.demo(now: demoNow, signedIn: false),
    );
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    final card = find.byType(TwitchLiveCard);
    if (card.evaluate().isNotEmpty) {
      await t.tap(card.first, buttons: kSecondaryButton);
      await settle(t, 6);
      expect(
        find.byKey(const ValueKey('twitch-stream-menu-follow')),
        findsNothing,
      );
    }
    await _close(t);
  });

  test('LocalTwitchBackend has no follows', () async {
    final twitch = TwitchService(
      backend: const LocalTwitchBackend(),
      preferences: MemoryPreferences(),
      clock: () => demoNow,
    );
    await twitch.init();
    expect(await twitch.refreshFollowed(_login), isNull);
    expect(twitch.followsOnTwitch(_login), isNull);
  });
}
