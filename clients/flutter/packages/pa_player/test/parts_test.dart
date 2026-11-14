// SPDX-License-Identifier: MPL-2.0
// LIB-020: the mpv EDL URI that plays the parts of a split movie as one
// timeline. The real playback is checked in edl_libmpv_test.dart.

import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/src/engine/parts.dart' show mediaDisplayName;

void main() {
  test('one part is played as it is', () {
    expect(partsUri(['/m/Arrival.mkv']), '/m/Arrival.mkv');
    expect(
      partsUri(['http://127.0.0.1:9/t/1/a.mkv']),
      'http://127.0.0.1:9/t/1/a.mkv',
    );
    expect(() => partsUri([]), throwsArgumentError);
  });

  test('two local parts make an EDL with byte lengths', () {
    expect(
      partsUri(['/m/Heat.cd1.avi', '/m/Heat.cd2.avi']),
      'edl://%15%/m/Heat.cd1.avi;%15%/m/Heat.cd2.avi',
    );
  });

  test('lengths count UTF-8 bytes; separators in names are safe', () {
    const parts = [
      '/фильмы/Чужой; часть 1,2 %20.vob', // ; , % and Cyrillic
      'http://127.0.0.1:4000/tok/3/%D0%A7/VTS_01_2.VOB',
    ];
    final uri = partsUri(parts);
    expect(uri, startsWith('edl://%48%/фильмы/'));
    expect(partsOf(uri), parts);
  });

  test('a plain URI is its own part; broken EDLs are null', () {
    expect(partsOf('/m/a.mkv'), ['/m/a.mkv']);
    expect(partsOf('edl://'), isNull);
    expect(partsOf('edl://%99%/short'), isNull);
    expect(partsOf('edl:///m/a.mkv'), isNull);
    expect(partsOf('edl://%3%abc,x'), isNull);
  });

  test('display name of a part list', () {
    expect(mediaDisplayName('/m/Heat%20one.mkv'), 'Heat one.mkv');
    expect(mediaDisplayName('/m/100%.mkv'), '100%.mkv');
    expect(
      mediaDisplayName(partsUri(['/m/Heat.cd1.avi', '/m/Heat.cd2.avi'])),
      'Heat.cd1.avi (+1)',
    );
    expect(mediaDisplayName('edl://broken'), 'EDL');
  });

  // LIB-021: subtitles of the parts on the common timeline.
  Duration s(num seconds) => Duration(milliseconds: (seconds * 1000).round());
  const parts = ['/m/Heat.cd1.avi', '/m/Heat.cd2.avi', '/m/Heat.cd3.avi'];

  test('part lengths come from the chapters mpv makes for the parts', () {
    expect(
      partLengths(parts, [
        (parts[0], s(0)),
        ('Opening', s(30)), // a chapter of the file itself
        (parts[1], s(3000)),
        (parts[2], s(5400)),
      ], s(7000)),
      [s(3000), s(2400), s(1600)],
    );
    // A part without its chapter, a timeline not starting at 0, starts
    // out of order or past the end: unknown.
    expect(
      partLengths(parts, [(parts[0], s(0)), (parts[2], s(10))], s(20)),
      isNull,
    );
    expect(
      partLengths(parts.sublist(0, 2), [
        (parts[0], s(1)),
        (parts[1], s(10)),
      ], s(20)),
      isNull,
    );
    expect(
      partLengths(parts.sublist(0, 2), [
        (parts[1], s(0)),
        (parts[0], s(10)),
      ], s(20)),
      isNull,
    );
    expect(
      partLengths(parts.sublist(0, 2), [
        (parts[0], s(0)),
        (parts[1], s(10)),
      ], s(10)),
      isNull,
    );
  });

  test('subtitles of the parts make one EDL track', () {
    final lengths = [s(3000), s(2400), s(1600)];
    expect(
      partSubtitlesUri(['/m/a.cd1.srt', '/m/a.cd2.srt', null], lengths),
      'edl://%12%/m/a.cd1.srt,length=3000.000;'
      '%12%/m/a.cd2.srt,length=2400.000',
    );
    // Only part 2 has subtitles: its segment reaches back to 0 with a
    // negative start, so its own 0 is where part 2 begins.
    expect(
      partSubtitlesUri([null, '/m/a.cd2.srt', null], lengths),
      'edl://%12%/m/a.cd2.srt,start=-3000.000,length=5400.000',
    );
    // A gap in the middle.
    expect(
      partSubtitlesUri(['/m/a.cd1.srt', null, '/m/a.cd3.srt'], lengths),
      'edl://%12%/m/a.cd1.srt,length=3000.000;'
      '%12%/m/a.cd3.srt,start=-2400.000,length=4000.000',
    );
    // Only part 1: the file itself; none: nothing.
    expect(
      partSubtitlesUri(['/m/a.cd1.srt', null, null], lengths),
      '/m/a.cd1.srt',
    );
    expect(partSubtitlesUri([null, null, null], lengths), isNull);
    expect(() => partSubtitlesUri(['/x'], lengths), throwsArgumentError);
    // Byte lengths: a URL with `,` and `;` stays one segment.
    final uri = partSubtitlesUri(
      ['http://h/t/1/%D0%A7;a,b.srt', 'http://h/t/1/b.srt'],
      [s(1.5), s(2)],
    )!;
    expect(
      uri,
      startsWith('edl://%27%http://h/t/1/%D0%A7;a,b.srt,length=1.500;'),
    );
  });
}
