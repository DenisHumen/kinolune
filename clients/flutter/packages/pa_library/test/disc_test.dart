// SPDX-License-Identifier: MPL-2.0
//
// PLAY-017: the angles of a disc title as player URIs and its other
// titles as cards of «Дополнительно».

import 'package:pa_library/pa_library.dart';
import 'package:test/test.dart';

void main() {
  const main = DiscTitle(
    id: '00001',
    durationMs: 5400000,
    angles: [
      ['/d/BDMV/STREAM/00001.m2ts', '/d/BDMV/STREAM/00002.m2ts'],
      ['/d/BDMV/STREAM/00001.m2ts', '/d/BDMV/STREAM/00003.m2ts'],
    ],
  );

  test('each angle plays as one URI', () {
    expect(main.angleUris, [
      'edl://%25%/d/BDMV/STREAM/00001.m2ts;%25%/d/BDMV/STREAM/00002.m2ts',
      'edl://%25%/d/BDMV/STREAM/00001.m2ts;%25%/d/BDMV/STREAM/00003.m2ts',
    ]);
    expect(discUri(['http://127.0.0.1:1/t/1/Film.iso/BDMV/STREAM/1.m2ts']), [
      'http://127.0.0.1:1/t/1/Film.iso/BDMV/STREAM/1.m2ts',
    ].single);
    // Lengths count UTF-8 bytes.
    expect(discUri(['/д/1', '/д/2']), 'edl://%5%/д/1;%5%/д/2');
  });

  test('extras are cards of their first angle', () {
    const disc = DiscPlayback(
      bluRay: true,
      main: main,
      extras: [
        DiscTitle(
          id: '00002',
          durationMs: 600000,
          angles: [
            ['/d/BDMV/STREAM/00010.m2ts'],
          ],
        ),
        DiscTitle(id: '00003', durationMs: 300000, angles: [[]]),
      ],
    );
    final cards = disc.extraCards;
    expect(cards, hasLength(1));
    expect(cards.single.title, '00002');
    expect(cards.single.kind, MediaExtraKind.discTitle);
    expect(cards.single.playbackUri, '/d/BDMV/STREAM/00010.m2ts');
    expect(cards.single.durationMs, 600000);
  });

  test('a protected disc has its code', () {
    expect(Unplayable.fromCode('protected_disc'), Unplayable.protectedDisc);
    expect(const DiscPlayback(bluRay: true, protection: 'aacs').protection,
        'aacs');
  });
}
