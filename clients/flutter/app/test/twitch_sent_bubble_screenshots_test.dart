// SPDX-License-Identifier: MPL-2.0
//
// TWI-039: frames of the "Sent" bubble with real fonts, every 40 ms over
// its 3 s, for the animation clips of the PR (computer and phone). Runs
// only with KINOLUNE_SCREENSHOTS_DIR set:
//   KINOLUNE_SCREENSHOTS_DIR=/tmp/twi039 \
//     flutter test test/twitch_sent_bubble_screenshots_test.dart

import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/sent_bubble.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart' show loadAppFonts, setSurface;

final _dir = Platform.environment['KINOLUNE_SCREENSHOTS_DIR'] ?? '';
const _key = ValueKey('sent-bubble-frame');

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(560, 300)),
    ('phone', FormFactor.phone, const Size(390, 300)),
  ]) {
    testWidgets('frames on $name', (t) async {
      setSurface(t, size, dpr: 2);
      final b = SentBubbles();
      addTearDown(b.dispose);
      await t.pumpWidget(
        FormFactorScope(
          formFactor: ff,
          child: MaterialApp(
            debugShowCheckedModeBanner: false,
            locale: const Locale('ru'),
            supportedLocales: AppLocalizations.supportedLocales,
            localizationsDelegates: kLocalizationsDelegates,
            theme: buildKinoluneTheme(ff),
            home: RepaintBoundary(
              key: _key,
              child: Stack(
                children: [
                  const Positioned.fill(
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        gradient: LinearGradient(
                          begin: Alignment.topLeft,
                          end: Alignment.bottomRight,
                          colors: [Color(0xFF243247), Color(0xFF0B0D12)],
                        ),
                      ),
                    ),
                  ),
                  Positioned.fill(
                    child: SentBubbleLayer(
                      bubbles: b,
                      formFactor: ff,
                      reduceMotion: false,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      await t.pump(const Duration(milliseconds: 400));
      b.add(
        SentBubbleData(
          kind: SentBubbleKind.sent,
          pieces: const [SentPiece.text('Вот это камбэк! GG, ребята')],
          avatar: const DecoratedBox(
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: Color(0xFF9146FF),
            ),
          ),
          seed: 3,
        ),
      );
      await t.pump();
      final boundary =
          t.renderObject(find.byKey(_key)) as RenderRepaintBoundary;
      for (var i = 0; i <= 80; i++) {
        await t.pump(Duration(milliseconds: i == 0 ? 0 : 40));
        final bytes = await t.runAsync(() async {
          final ui.Image image = await boundary.toImage(pixelRatio: 2);
          final data = await image.toByteData(format: ui.ImageByteFormat.png);
          image.dispose();
          return data!.buffer.asUint8List();
        });
        final file = File(
          '$_dir/$name/frame_${i.toString().padLeft(3, '0')}.png',
        );
        file.parent.createSync(recursive: true);
        file.writeAsBytesSync(bytes!);
      }
      await t.pump(const Duration(seconds: 1));
    }, skip: _dir.isEmpty);
  }
}
