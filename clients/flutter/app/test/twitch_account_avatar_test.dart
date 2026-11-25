// SPDX-License-Identifier: MPL-2.0
//
// TWI-032: the Twitch account's name and avatar in the app. Fake Helix
// (`GET /helix/users` with the user token): after sign-in the name and the
// avatar are in the Streams header, in Settings → Twitch and on the active
// profile; a new avatar on Twitch shows after an update; the 24-hour rule;
// a sign-in Twitch no longer accepts signs out; a profile's own picture
// and a kid profile stay; after signing out the previous avatar is back;
// a second device with the synced profile shows the same picture.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_profiles.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/profiles/profile_avatar.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:kinolune/widgets/round_photo.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _avatarV1 = 'https://static-cdn.example/u/7999-profile_image-300x300.png';
const _avatarV2 = 'https://static-cdn.example/u/7999-new-profile_image.png';

FakeTwitchBackend _helix({bool signedIn = false, String avatar = _avatarV1}) =>
    FakeTwitchBackend.demo(now: demoNow, signedIn: signedIn)
      ..me = TwitchChannel(
        userId: '7999',
        login: 'demo_viewer',
        displayName: 'Demo_Viewer',
        avatarUrl: avatar,
      );

Future<void> _signIn(TwitchService twitch) async {
  await twitch.startLogin();
  await twitch.pollLogin();
}

Future<String?> _avatar(FakeProfiles p, int id) async =>
    (await p.profiles()).firstWhere((e) => e.id == id).avatar;

void main() {
  group('avatar reference', () {
    test('Twitch avatar keeps the previous colour or picture', () {
      final a = AvatarRef.parse('icon:3:4').withTwitch('7999', _avatarV1);
      expect(a.ref, 'twitch:7999;icon:3:4;$_avatarV1');
      final back = AvatarRef.parse(a.ref);
      expect(back, a);
      expect(back.isTwitch, isTrue);
      expect(back.base.ref, 'icon:3:4');
      expect(AvatarRef.parse('twitch:7999;color:2').twitchUrl, isNull);
      expect(AvatarRef.parse('twitch:;color:2').isTwitch, isFalse);
    });

    test('a URL too long for the core is left out', () {
      final long = 'https://x.example/${'a' * 300}';
      final a = AvatarRef.parse('color:1').withTwitch('7999', long);
      expect(a.ref, 'twitch:7999;color:1');
      expect(a.ref.length, lessThanOrEqualTo(AvatarRef.maxLength));
    });

    test('own colour or picture drops the Twitch avatar', () {
      final a = AvatarRef.parse('twitch:7999;color:1;$_avatarV1');
      expect(a.withColor(3).isTwitch, isFalse);
      expect(a.withIcon(2).ref, 'icon:2:1');
      expect(isStandardAvatar('color:0'), isTrue);
      expect(isStandardAvatar('icon:1:2'), isTrue);
      expect(isStandardAvatar(null), isTrue);
      expect(isStandardAvatar('photo:abc'), isFalse);
      expect(isStandardAvatar('twitch:1;color:0'), isFalse);
    });
  });

  group('own profile from Helix', () {
    test('after sign-in: name and avatar, kept without a token', () async {
      final helix = _helix();
      final prefs = MemoryPreferences();
      final twitch = TwitchService(
        backend: helix,
        preferences: prefs,
        clock: () => demoNow,
      );
      await twitch.init();
      expect(twitch.accountProfile, isNull);
      await _signIn(twitch);
      expect(twitch.accountName, 'Demo_Viewer');
      expect(twitch.accountProfile?.avatarUrl, _avatarV1);
      final saved = await prefs.read(Preferences.twitchAccountProfile);
      expect(saved, contains('Demo_Viewer'));
      expect(saved, isNot(contains('token')));

      // Another start reads it without asking Helix.
      final again = TwitchService(
        backend: helix,
        preferences: prefs,
        clock: () => demoNow.add(const Duration(hours: 1)),
      );
      helix.calls.clear();
      await again.init();
      await again.refreshAccount();
      expect(again.accountName, 'Demo_Viewer');
      expect(helix.calls, isNot(contains('users')));
    });

    test('at most once a day; an update brings a new avatar', () async {
      final helix = _helix();
      var now = demoNow;
      final twitch = TwitchService(
        backend: helix,
        preferences: MemoryPreferences(),
        clock: () => now,
      );
      await twitch.init();
      await _signIn(twitch);
      helix
        ..me = const TwitchChannel(
          userId: '7999',
          login: 'demo_viewer',
          displayName: 'DemoViewer2',
          avatarUrl: _avatarV2,
        )
        ..calls.clear();

      now = demoNow.add(const Duration(hours: 23));
      await twitch.refreshAccount();
      expect(helix.calls, isEmpty);
      expect(twitch.accountProfile?.avatarUrl, _avatarV1);

      // "Update name and avatar".
      await twitch.refreshAccount(force: true);
      expect(twitch.accountProfile?.avatarUrl, _avatarV2);
      expect(twitch.accountName, 'DemoViewer2');

      helix.me = const TwitchChannel(
        userId: '7999',
        login: 'demo_viewer',
        displayName: 'DemoViewer3',
      );
      now = now.add(const Duration(hours: 25));
      await twitch.refresh();
      expect(twitch.accountName, 'DemoViewer3');
      expect(twitch.accountProfile?.avatarUrl, isNull);
    });

    test('Twitch no longer accepts the sign-in: signed out', () async {
      final helix = _helix();
      final twitch = TwitchService(
        backend: helix,
        preferences: MemoryPreferences(),
        clock: () => demoNow,
      );
      String? left;
      twitch.onSignedOut = (id) async => left = id;
      await twitch.init();
      await _signIn(twitch);
      helix.expired = true;
      await twitch.refreshAccount(force: true);
      expect(twitch.account, isNull);
      expect(twitch.accountProfile, isNull);
      expect(left, '7999');
    });

  });

  group('profile avatar', () {
    test('set on the active profile, restored after sign-out', () async {
      final profiles = FakeProfiles.demo();
      final services = await demoServices(
        twitch: _helix(),
        profiles: profiles,
      );
      await _signIn(services.twitch);
      final alex = AvatarRef.parse(await _avatar(profiles, 1));
      expect(alex.twitchUserId, '7999');
      expect(alex.twitchUrl, _avatarV1);
      expect(alex.base.ref, 'color:0');
      // Not the other profiles, not the kid.
      expect(await _avatar(profiles, 2), 'icon:3:4');
      expect(await _avatar(profiles, 3), 'icon:1:2');

      // A new picture on Twitch reaches the profile.
      (services.twitch.backend as FakeTwitchBackend).me = const TwitchChannel(
        userId: '7999',
        login: 'demo_viewer',
        displayName: 'Demo_Viewer',
        avatarUrl: _avatarV2,
      );
      await services.twitch.refreshAccount(force: true);
      expect(AvatarRef.parse(await _avatar(profiles, 1)).twitchUrl, _avatarV2);

      await services.twitch.signOut();
      expect(await _avatar(profiles, 1), 'color:0');
    });

    test('own picture and a kid profile are not changed', () async {
      final profiles = FakeProfiles(
        profiles: const [
          LibraryProfile(
            id: 1,
            syncId: 'default',
            name: 'Alex',
            avatar: 'photo:holiday',
          ),
        ],
      );
      final services = await demoServices(
        twitch: _helix(),
        profiles: profiles,
      );
      await _signIn(services.twitch);
      expect(await _avatar(profiles, 1), 'photo:holiday');

      final kids = FakeProfiles.demo(kidActive: true);
      final kidServices = await demoServices(
        twitch: _helix(),
        profiles: kids,
      );
      await _signIn(kidServices.twitch);
      expect(await _avatar(kids, 3), 'icon:1:2');
    });

    testWidgets('second device shows the synced Twitch avatar', (t) async {
      // The profile as it arrives through the profile sync: no account,
      // no token on this device.
      const synced = LibraryProfile(
        id: 1,
        syncId: 'default',
        name: 'Alex',
        avatar: 'twitch:7999;color:0;demo:',
      );
      await t.pumpWidget(
        MaterialApp(home: Center(child: ProfileAvatar.of(synced, size: 56))),
      );
      final photo = t.widget<RoundPhoto>(find.byType(RoundPhoto));
      expect(photo.url, 'demo:');
      expect(find.byKey(const ValueKey('profile-avatar-twitch')), findsOneWidget);
    });
  });

  group('screens', () {
    for (final (form, size) in [
      (FormFactor.desktop, const Size(1280, 800)),
      (FormFactor.phone, const Size(390, 844)),
    ]) {
      testWidgets('Streams header shows name and avatar (${form.name})', (
        t,
      ) async {
        setSurface(t, size);
        final helix = _helix(signedIn: true, avatar: 'demo:');
        final services = await demoServices(twitch: helix);
        await services.setShowTwitch(true);
        await t.pumpWidget(
          testApp(services, section: Section.twitch, formFactor: form),
        );
        await settle(t);
        final name = find.byKey(const ValueKey('twitch-account-name'));
        expect(name, findsOneWidget);
        expect(t.widget<Text>(name).data, 'Demo_Viewer');
        expect(
          find.byKey(const ValueKey('twitch-account-avatar')),
          findsOneWidget,
        );
        await capture(t, 'twi-032-streams-${form.name}');

        // The menu updates the profile from Twitch.
        helix.me = const TwitchChannel(
          userId: '7999',
          login: 'demo_viewer',
          displayName: 'Viewer_Renamed',
          avatarUrl: 'demo:',
        );
        await t.tap(find.byKey(const ValueKey('twitch-account')));
        await settle(t, 4);
        await t.tap(find.byKey(const ValueKey('twitch-account-refresh')));
        await settle(t, 6);
        expect(find.text('Viewer_Renamed'), findsOneWidget);
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });
    }

    testWidgets('Settings → Twitch: the account card', (t) async {
      setSurface(t, const Size(1280, 800));
      final services = await demoServices(
        twitch: _helix(signedIn: true, avatar: 'demo:'),
        advancedSettings: true,
      );
      await services.twitch.refreshAccount(force: true);
      await t.pumpWidget(testApp(services, section: Section.settings));
      await settle(t, 8);
      final card = find.byKey(const ValueKey('settings-twitch-account'));
      final page = find
          .descendant(
            of: find.byType(SettingsScreen),
            matching: find.byType(Scrollable),
          )
          .first;
      await t.scrollUntilVisible(card, 200, scrollable: page);
      await settle(t, 3);
      expect(
        find.descendant(of: card, matching: find.text('Demo_Viewer')),
        findsOneWidget,
      );
      await capture(t, 'twi-032-settings-desktop');
      await t.tap(find.byKey(const ValueKey('settings-twitch-sign-out')));
      await settle(t, 4);
      expect(services.twitch.account, isNull);
      expect(
        find.byKey(const ValueKey('settings-twitch-sign-in')),
        findsOneWidget,
      );
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });
}
