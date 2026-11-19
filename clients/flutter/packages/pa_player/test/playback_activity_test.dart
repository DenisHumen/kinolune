// SPDX-License-Identifier: MPL-2.0
//
// PKG-024: silent updates wait while anything plays or a video is open.

import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';

void main() {
  tearDown(PlaybackActivity.reset);

  test('busy while any engine plays or holds a video', () {
    final music = Object();
    final video = Object();
    expect(PlaybackActivity.busy.value, isFalse);
    PlaybackActivity.report(music, playing: true);
    expect(PlaybackActivity.busy.value, isTrue);
    PlaybackActivity.report(music, playing: false);
    expect(PlaybackActivity.busy.value, isFalse, reason: 'paused music');
    PlaybackActivity.report(video, playing: false, holding: true);
    expect(PlaybackActivity.busy.value, isTrue, reason: 'a paused film');
    PlaybackActivity.forget(video);
    expect(PlaybackActivity.busy.value, isFalse);
  });

  test('listeners fire on changes only', () {
    final e = Object();
    var calls = 0;
    void listener() => calls++;
    PlaybackActivity.busy.addListener(listener);
    addTearDown(() => PlaybackActivity.busy.removeListener(listener));
    PlaybackActivity.report(e, playing: true);
    PlaybackActivity.report(e, playing: true);
    PlaybackActivity.report(e, playing: true, holding: true);
    expect(calls, 1);
    PlaybackActivity.forget(e);
    expect(calls, 2);
  });
}
