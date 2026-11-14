// SPDX-License-Identifier: MPL-2.0
//
// Profile screens (UI-015) for the l10n layout test and the screenshots:
// the start picker, the Settings gate of a kid profile, profile management,
// the kid profile editor and App Lock, on in-memory profiles.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_profiles.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'test_app.dart';

typedef ProfileStep = Future<void> Function(WidgetTester t, AppServices s);

class ProfileFlow {
  const ProfileFlow({
    this.kid = false,
    this.picker = false,
    this.appLock = false,
    this.step,
  });

  final bool kid;
  final bool picker;
  final bool appLock;
  final ProfileStep? step;

  Future<AppServices> services() {
    final prefs = MemoryPreferences();
    if (!picker) prefs.values[Preferences.profilesPickerOnStart] = 'false';
    if (appLock) prefs.values[Preferences.profilesAppLock] = 'true';
    return demoServices(
      preferences: prefs,
      profiles: FakeProfiles.demo(kidActive: kid),
    );
  }
}

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = find.byKey(ValueKey(key)).last;
  await t.ensureVisible(f);
  await settle(t, 2);
  await t.tap(f);
  await settle(t, 6);
}

Future<void> _pin(WidgetTester t, String pin) async {
  for (final d in pin.split('')) {
    await t.tap(find.byKey(ValueKey('pin-key-$d')).last);
    await t.pump();
  }
  await t.tap(find.byKey(const ValueKey('pin-key-ok')).last);
  await settle(t, 6);
}

Future<void> _settings(WidgetTester t) async {
  t.state<AppShellState>(find.byType(AppShell)).select(Section.settings);
  await settle(t, 6);
}

Future<void> _manage(WidgetTester t, AppServices s) async {
  await _settings(t);
  await _tapKey(t, 'settings-profiles-manage');
  await _pin(t, '1234');
}

final profileFlows = <String, ProfileFlow>{
  'profiles-picker': const ProfileFlow(kid: true, picker: true),
  'profiles-gate': ProfileFlow(kid: true, step: (t, _) => _settings(t)),
  'profiles-gate-wrong': ProfileFlow(
    kid: true,
    step: (t, _) async {
      await _settings(t);
      await _pin(t, '0000');
    },
  ),
  'profiles-settings': ProfileFlow(step: (t, _) => _settings(t)),
  'profiles-manage': ProfileFlow(step: _manage),
  'profiles-editor-kid': ProfileFlow(
    step: (t, s) async {
      await _manage(t, s);
      await _tapKey(t, 'profiles-add-kid');
    },
  ),
  'profiles-leave-kid': ProfileFlow(
    kid: true,
    picker: true,
    step: (t, _) => _tapKey(t, 'profile-pick-2'),
  ),
  'profiles-app-lock': const ProfileFlow(appLock: true),
};
