// SPDX-License-Identifier: MPL-2.0
//
// UI-032: a frame of the file itself instead of the coloured tile — for
// the videos of an open personal library (never asked for while it is
// locked) and for «Мои видео». The frames here are drawn in code. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-032 \
//     flutter test test/personal_frames_test.dart

import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_personal_media.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/own_videos.dart' show OwnVideosScreen;
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _typePin(WidgetTester t, String pin) async {
  for (final c in pin.split('')) {
    await t.sendKeyEvent(
      LogicalKeyboardKey.findKeyByKeyId(0x30 + int.parse(c))!,
    );
  }
  await t.sendKeyEvent(LogicalKeyboardKey.enter);
  await settle(t, 4);
}

/// A 16:9 "still" drawn in code: sky, sun, hills or sea, by [seed].
Future<Uint8List> _still(int seed) async {
  const w = 480.0, h = 270.0;
  final rec = ui.PictureRecorder();
  final c = Canvas(rec);
  final hue = (seed * 53 + 190) % 360.0;
  final sky = Paint()
    ..shader = ui.Gradient.linear(const Offset(0, 0), const Offset(0, h), [
      HSLColor.fromAHSL(1, hue, .55, .62).toColor(),
      HSLColor.fromAHSL(1, (hue + 30) % 360, .65, .82).toColor(),
    ]);
  c.drawRect(const Rect.fromLTWH(0, 0, w, h), sky);
  c.drawCircle(
    Offset(90.0 + seed * 57 % 300, 70),
    28,
    Paint()..color = const Color(0xFFFFF1C4),
  );
  for (var layer = 0; layer < 3; layer++) {
    final y = h * (.55 + layer * .13);
    final path = Path()..moveTo(0, y);
    for (var x = 0.0; x <= w; x += 40) {
      final bump = ((x / 40 + seed + layer * 3) % 5 - 2) * (8 - layer * 2);
      path.lineTo(x, y + bump);
    }
    path
      ..lineTo(w, h)
      ..lineTo(0, h)
      ..close();
    c.drawPath(
      path,
      Paint()
        ..color = HSLColor.fromAHSL(
          1,
          (hue + 120 + layer * 12) % 360,
          .35,
          .38 - layer * .1,
        ).toColor(),
    );
  }
  final image = await rec.endRecording().toImage(w.toInt(), h.toInt());
  final data = await image.toByteData(format: ui.ImageByteFormat.png);
  image.dispose();
  return data!.buffer.asUint8List();
}

Future<Map<int, Uint8List>> _stills(WidgetTester t, Iterable<int> ids) async =>
    (await t.runAsync(
      () async => {for (final id in ids) id: await _still(id)},
    ))!;

/// Lets the images decode (real async work), then draws.
Future<void> _decode(WidgetTester t) async {
  await t.runAsync(
    () => Future<void>.delayed(const Duration(milliseconds: 300)),
  );
  await settle(t, 3);
}

Future<FakePersonalMedia> _openFamily(
  WidgetTester t, {
  Size size = const Size(1280, 800),
  FormFactor formFactor = FormFactor.desktop,
  bool withFrames = true,
}) async {
  setSurface(t, size);
  final personal = FakePersonalMedia.demo();
  // Every video of «Семья» but «Первый снег» (4) has a frame.
  if (withFrames) personal.frames.addAll(await _stills(t, [1, 2, 3, 5]));
  final s = await demoServices(personal: personal);
  s.locale.value = const Locale('ru');
  await t.pumpWidget(
    testApp(s, formFactor: formFactor, section: Section.personal),
  );
  await settle(t);
  // Locked: nothing of it is asked for.
  expect(personal.frameCalls, isEmpty);
  await t.tap(_key('personal-library-1'));
  await settle(t, 4);
  await _typePin(t, '2468');
  expect(_key('personal-library-page-1'), findsOneWidget);
  await _decode(t);
  return personal;
}

List<OwnVideo> _clips() => [
  for (final (i, name, minutes) in [
    (1, 'VID_20240612_183000', 2),
    (2, 'VID_20240613_101500', 1),
    (3, 'PXL_20240701_090000123', 4),
    (4, 'IMG_4521', 1),
  ])
    OwnVideo(
      fileId: i,
      sourceId: 1,
      relPath: 'Телефон/DCIM/Camera/$name.mp4',
      localPath: '/media/Телефон/DCIM/Camera/$name.mp4',
      title: name,
      sizeBytes: 40 << 20,
      durationMs: minutes * 60 * 1000,
      takenAt: DateTime(2024, 6, 12 + i, 18, 30),
    ),
];

void main() {
  setUpAll(loadAppFonts);

  testWidgets('a tile of an open library shows the frame of its file', (
    t,
  ) async {
    final personal = await _openFamily(t);
    // The grid and the «Продолжить» row show the frame.
    expect(_key('personal-video-frame-1'), findsOneWidget);
    expect(_key('personal-continue-frame-5'), findsOneWidget);
    final image = t.widget<Image>(_key('personal-video-frame-1'));
    expect(image.fit, BoxFit.cover);
    expect(
      (image.image as MemoryImage).bytes,
      same(personal.frames[1]),
      reason: 'the bytes from the core, not a path',
    );
    // No frame for the file: the tile keeps its tint.
    expect(_key('personal-video-4'), findsOneWidget);
    expect(_key('personal-video-frame-4'), findsNothing);
    expect(personal.frameCalls, containsAll([1, 2, 3, 4, 5]));
    await capture(t, '01-library-frames-desktop');

    // Locked: the page closes and no frame is asked for any more.
    await t.tap(_key('personal-lock-now'));
    await settle(t, 4);
    expect(_key('personal-library-page-1'), findsNothing);
    expect(_key('personal-video-frame-1'), findsNothing);
    final asked = personal.frameCalls.length;
    await settle(t, 4);
    expect(personal.frameCalls.length, asked);
  });

  testWidgets('a locked library refuses its frames', (t) async {
    final personal = FakePersonalMedia.demo();
    personal.frames.addAll(await _stills(t, [1]));
    final video = PersonalVideo(
      fileId: 1,
      sourceId: 7,
      relPath: 'Видео/Дача, июль.mp4',
      title: 'Дача, июль',
      addedAt: DateTime(2026, 11, 14),
    );
    await expectLater(personal.frame(1, video), throwsA(isA<PersonalLocked>()));
    expect(personal.frameCalls, isEmpty);
  });

  testWidgets('frames on the phone', (t) async {
    await _openFamily(
      t,
      size: const Size(412, 892),
      formFactor: FormFactor.phone,
    );
    expect(_key('personal-video-frame-1'), findsOneWidget);
    await capture(t, '02-library-frames-phone');
  });

  testWidgets('«Мои видео» show the frames of the clips', (t) async {
    setSurface(t, const Size(1280, 800));
    final library = FakeLibrary(scanStepDelay: Duration.zero)
      ..ownVideoList.addAll(_clips())
      ..ownVideoFrames.addAll(
        await _stills(t, [
          11,
          13,
          14,
        ]).then((m) => {for (final e in m.entries) e.key - 10: e.value}),
      );
    final s = await demoServices(
      library: library,
      preferences: MemoryPreferences()
        ..values[Preferences.onboardingDone] = 'true',
    );
    s.locale.value = const Locale('ru');
    await t.pumpWidget(testApp(s, section: Section.home));
    await settle(t, 6);
    await _decode(t);
    expect(_key('home-own-videos'), findsOneWidget);
    expect(_key('own-video-frame-1'), findsOneWidget);
    // VID_20240613_101500 has none: the placeholder stays.
    expect(_key('own-video-frame-2'), findsNothing);
    expect(_key('own-video-2'), findsOneWidget);
    await capture(t, '03-own-videos-home');

    // Not awaited: the route completes only when it is popped.
    Navigator.of(t.element(_key('home-own-videos')))
        .push(MaterialPageRoute<void>(builder: (_) => const OwnVideosScreen()))
        .ignore();
    await settle(t, 6);
    await _decode(t);
    expect(_key('own-videos-grid'), findsOneWidget);
    expect(_key('own-video-frame-3'), findsOneWidget);
    await capture(t, '04-own-videos-all');
  });
}
