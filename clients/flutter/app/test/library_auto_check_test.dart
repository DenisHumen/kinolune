// SPDX-License-Identifier: MPL-2.0
//
// LIB-024: the app asks the library for a check when it comes back to the
// foreground and when the network returns; services pass it on to a
// library that recognises on its own.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/services/library_auto_check.dart';

import 'support/test_app.dart';

void main() {
  test('resume checks sources not listed for two minutes', () {
    final calls = <(Duration, bool)>[];
    final c = LibraryAutoCheck(({
      Duration minAge = Duration.zero,
      bool networkBack = false,
    }) async {
      calls.add((minAge, networkBack));
    });
    c.resumed();
    expect(calls, [(LibraryAutoCheck.resumeMinAge, false)]);
  });

  test('only a network that comes back after being offline checks', () {
    final calls = <(Duration, bool)>[];
    final c = LibraryAutoCheck(({
      Duration minAge = Duration.zero,
      bool networkBack = false,
    }) async {
      calls.add((minAge, networkBack));
    });
    c.network(online: true); // first report: the app just started
    c.network(online: true); // Wi-Fi to Ethernet
    expect(calls, isEmpty);
    c.network(online: false);
    expect(calls, isEmpty);
    c.network(online: true);
    expect(calls, [(Duration.zero, true)]);
  });

  testWidgets('services pass checks on to the library and show its status', (t) async {
    final lib = FakeLibrary.demo(scanStepDelay: Duration.zero);
    final s = await demoServices(library: lib);
    await Future<void>.delayed(Duration.zero);
    expect(lib.watching, isTrue);
    await s.checkLibraryNow(networkBack: true, minAge: Duration.zero);
    expect(lib.checks.single.networkBack, isTrue);
    expect(lib.checks.single.minAge, Duration.zero);
    lib.recognitionStatus = lib.recognitionStatus.copyWith(worksWaiting: 4);
    await Future<void>.delayed(Duration.zero);
    expect(s.recognitionStatus.value?.worksWaiting, 4);
  });
}
