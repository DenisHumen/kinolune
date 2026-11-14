// SPDX-License-Identifier: MPL-2.0
//
// LIB-012: the local profile of the first-run setup (UI-004, kept in
// Preferences) becomes the default profile of the Rust core once; later
// changes go to the core's active profile.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/local_profile.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:pa_player/testing.dart';

import 'support/native_library.dart';
import 'support/test_app.dart' show demoDiscovery;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Directory tmp;
  late RustLibrary lib;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() async {
    tmp = Directory.systemTemp.createTempSync('kinolune_profiles_');
    lib = await RustLibrary.open('${tmp.path}/db/core.db');
  });

  tearDown(() async {
    await lib.close();
    tmp.deleteSync(recursive: true);
  });

  Future<AppServices> services(Preferences prefs) async {
    final s = AppServices(
      library: lib,
      secureStore: MemorySecureStore(),
      preferences: prefs,
      playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
      sourceManager: FakeSourceManager(lib),
      discovery: demoDiscovery,
      sync: FakeDeviceSync.demo(),
      clock: () => DateTime(2026, 11, 14),
      imageCache: demoImageCache(),
    );
    await s.init();
    return s;
  }

  test('the UI-004 profile moves into the default core profile', () async {
    final prefs = MemoryPreferences();
    await prefs.write(
      Preferences.localProfile,
      jsonEncode(const LocalProfile(name: 'Alex', avatar: 3).toJson()),
    );
    final s = await services(prefs);
    expect(s.profiles, isNotNull);
    expect(s.profile.value, const LocalProfile(name: 'Alex', avatar: 3));
    final core = await lib.activeProfile();
    expect(core.syncId, 'default');
    expect(core.name, 'Alex');
    expect(core.avatar, 'color:3');
    expect(await prefs.read(Preferences.localProfile), isNull);

    // Renaming in the app renames the core profile.
    await s.setProfile(const LocalProfile(name: 'Sasha', avatar: 1));
    expect((await lib.activeProfile()).name, 'Sasha');
    final again = await services(MemoryPreferences());
    expect(again.profile.value, const LocalProfile(name: 'Sasha', avatar: 1));
  });

  test('a named core profile is not overwritten', () async {
    final first = await lib.activeProfile();
    await lib.updateProfile(
      first.id,
      first.settings.copyWith(name: 'Parent', avatar: 'color:2'),
    );
    final prefs = MemoryPreferences();
    await prefs.write(
      Preferences.localProfile,
      jsonEncode(const LocalProfile(name: 'Old', avatar: 5).toJson()),
    );
    final s = await services(prefs);
    expect(s.profile.value, const LocalProfile(name: 'Parent', avatar: 2));
    expect(await prefs.read(Preferences.localProfile), isNull);
  });
}
