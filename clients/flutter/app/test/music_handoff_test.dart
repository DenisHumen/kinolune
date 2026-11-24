// SPDX-License-Identifier: MPL-2.0
//
// MUS-015: music continues on another device. The handoff payload (whole
// queue, current track, playhead; no history), trimming of long queues,
// the resume position on the target, finding tracks in the target's own
// library, and «Продолжить на…» / «Забрать сюда» between two paired fakes.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/features/music/music_handoff.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

PlayRequest _t(
  String id,
  String title, {
  String artist = 'Band',
  String album = 'Album',
  int durationMs = 200000,
  String? uri,
}) => PlayRequest(
  uri: uri ?? '/phone/music/$title.flac',
  title: title,
  itemId: id,
  artist: artist,
  album: album,
  durationMs: durationMs,
);

final _phoneTracks = [for (var i = 1; i <= 5; i++) _t('p$i', 'Song $i')];

/// The TV's library: the same songs under other ids and paths, one
/// missing (Song 4), one tagged a little differently (Song 5 ‹feat.›).
final _tvTracks = [
  for (var i = 1; i <= 3; i++)
    _t('tv$i', 'Song $i', uri: 'http://127.0.0.1:9000/tv$i'),
  _t('tv5', 'Song 5!', album: 'Album (Deluxe)', durationMs: 201500),
];

MusicPlayerController _player(FakePlayerEngine e) =>
    MusicPlayerController(engineFactory: ({bool video = true}) => e);

Future<(FakeDeviceSync, FakeDeviceSync)> _pair() async {
  final lan = FakeSyncLan();
  final tv = FakeDeviceSync(
    id: 'ktv',
    name: 'TV',
    kind: SyncDeviceKind.tv,
    lan: lan,
  );
  final phone = FakeDeviceSync(
    id: 'kphone',
    name: 'Phone',
    kind: SyncDeviceKind.phone,
    lan: lan,
  );
  await phone.join((await tv.invite()).code);
  return (tv, phone);
}

void main() {
  final t0 = DateTime.utc(2026, 11, 24, 12);

  group('payload', () {
    test('whole queue, current track and playhead; no history', () async {
      final e = FakePlayerEngine();
      final p = _player(e);
      await p.play(_phoneTracks, index: 2);
      e.advance(const Duration(seconds: 61));
      final payload = musicHandoffPayload(p, now: t0);
      expect(payload['v'], MusicQueue.jsonVersion);
      expect(payload['handoff'], musicHandoffVersion);
      expect((payload['tracks'] as List).length, 5);
      expect(payload['position'], 2);
      expect(payload['positionMs'], 61000);
      expect(payload['playing'], isTrue);
      expect(payload['sentAtMs'], t0.millisecondsSinceEpoch);
      expect(payload['recent'], isEmpty, reason: 'history stays here');
      expect(utf8.encode(jsonEncode(payload)).length, lessThan(4096));
    });

    test('a long queue keeps 20 tracks back and as many ahead as fit', () {
      final tracks = [
        for (var i = 0; i < 3000; i++)
          musicRequestToJson(_t('id$i', 'Song number $i with a long title')),
      ];
      final payload = <String, Object?>{
        'v': 1,
        'tracks': tracks,
        'unshuffled': [for (var i = 0; i < 3000; i++) i],
        'position': 1000,
        'positionMs': 5000,
      };
      final out = trimMusicHandoff(payload, maxBytes: 64 * 1024);
      expect(utf8.encode(jsonEncode(out)).length, lessThanOrEqualTo(65536));
      final kept = out['tracks'] as List;
      expect(out['position'], handoffHistoryWindow);
      expect(
        musicRequestFromJson(kept[out['position'] as int])!.itemId,
        'id1000',
        reason: 'the current track stays current',
      );
      expect(kept.length, greaterThan(100));
      expect(out.containsKey('unshuffled'), isFalse);
      // Short queues stay as they are.
      final small = {...payload, 'tracks': tracks.take(3).toList()}
        ..['position'] = 1;
      expect(trimMusicHandoff(small), same(small));
    });

    test('resume position: playhead plus time in flight, within the track', () {
      final base = <String, Object?>{
        'tracks': [musicRequestToJson(_t('a', 'A', durationMs: 100000))],
        'position': 0,
        'positionMs': 61000,
        'sentAtMs': t0.millisecondsSinceEpoch,
      };
      Duration at(Map<String, Object?> p, int afterMs) =>
          musicHandoffStart(p, t0.add(Duration(milliseconds: afterMs)));
      expect(at({...base, 'playing': true}, 1500).inMilliseconds, 62500);
      expect(at({...base, 'playing': false}, 1500).inMilliseconds, 61000);
      expect(
        at({...base, 'playing': true}, 10 * 60 * 1000).inMilliseconds,
        99000,
        reason: 'lag capped at 60 s and never past the end',
      );
      expect(
        at({...base, 'playing': true}, -5000).inMilliseconds,
        61000,
        reason: 'a target clock behind the source adds nothing',
      );
    });

    test("tracks are found in the target's own library", () {
      final index = HandoffTrackIndex(_tvTracks);
      expect(index.resolve(_t('p1', 'Song 1'))!.itemId, 'tv1');
      expect(index.resolve(_t('p4', 'Song 4')), isNull);
      expect(
        index.resolve(_t('p5', 'Song 5'))!.itemId,
        'tv5',
        reason: 'same artist and title (punctuation aside), ±3 s',
      );
      expect(
        index.resolve(_t('p5', 'Song 5', durationMs: 260000)),
        isNull,
        reason: 'another recording',
      );
      final stream = _t('s', 'Radio', uri: 'https://radio.example/stream');
      expect(index.resolve(stream), same(stream));
      expect(
        index.resolve(_t('l', 'Loop', uri: 'http://127.0.0.1:1234/x')),
        isNull,
        reason: "another device's loopback proxy",
      );
    });
  });

  group('between two devices', () {
    test('continue on the TV: same track, same second; phone pauses', () async {
      final (tv, phone) = await _pair();
      final phoneEngine = FakePlayerEngine();
      final tvEngine = FakePlayerEngine();
      final phonePlayer = _player(phoneEngine);
      final tvPlayer = _player(tvEngine);
      var now = t0;
      final onPhone = MusicHandoffController(
        sync: phone,
        player: phonePlayer,
        libraryTracks: () async => _phoneTracks,
        clock: () => now,
      )..start();
      final onTv = MusicHandoffController(
        sync: tv,
        player: tvPlayer,
        libraryTracks: () async => _tvTracks,
        clock: () => now.add(const Duration(milliseconds: 800)),
      )..start();
      addTearDown(onPhone.dispose);
      addTearDown(onTv.dispose);

      await phonePlayer.play(_phoneTracks, index: 1);
      phoneEngine.advance(const Duration(seconds: 61));
      expect(onPhone.targets().map((d) => d.id), ['ktv']);
      expect(await onPhone.continueOn(onPhone.targets().single), isTrue);
      await pumpEventQueue();

      expect(phonePlayer.playing, isFalse, reason: 'the source pauses');
      expect(phone.sentMusic.single.to, 'ktv');
      expect(tvPlayer.current!.itemId, 'tv2');
      expect(tvPlayer.playing, isTrue);
      expect(
        tvPlayer.position.inMilliseconds,
        61800,
        reason: '61 s plus 0.8 s in flight',
      );
      expect(tvPlayer.queue.ordered.map((r) => r.itemId), [
        'tv1',
        'tv2',
        'tv3',
        'tv5',
      ], reason: 'Song 4 is not on the TV');
      expect(tvEngine.uris.first, 'http://127.0.0.1:9000/tv1');
    });

    test('take it here: the TV asks, the phone answers and pauses', () async {
      final (tv, phone) = await _pair();
      final phoneEngine = FakePlayerEngine();
      final phonePlayer = _player(phoneEngine);
      final tvPlayer = _player(FakePlayerEngine());
      final onPhone = MusicHandoffController(
        sync: phone,
        player: phonePlayer,
        libraryTracks: () async => _phoneTracks,
        clock: () => t0,
      )..start();
      final onTv = MusicHandoffController(
        sync: tv,
        player: tvPlayer,
        libraryTracks: () async => _tvTracks,
        clock: () => t0,
        pullTimeout: const Duration(milliseconds: 200),
      )..start();
      addTearDown(onPhone.dispose);
      addTearDown(onTv.dispose);

      final phoneDevice = onTv.targets().single;
      // Nothing plays on the phone yet: no answer.
      expect(await onTv.pullFrom(phoneDevice), isFalse);

      await phonePlayer.play(_phoneTracks);
      phoneEngine.advance(const Duration(seconds: 30));
      expect(await onTv.pullFrom(phoneDevice), isTrue);
      expect(phonePlayer.playing, isFalse);
      expect(tvPlayer.current!.itemId, 'tv1');
      expect(tvPlayer.position.inMilliseconds, 30000);
      final answer = phone.sentMusic.last;
      expect(answer.to, 'ktv');
      expect(answer.session, startsWith('ktv-music-'));
    });

    test('a queue with nothing playable here keeps the own queue', () async {
      final (tv, phone) = await _pair();
      final tvPlayer = _player(FakePlayerEngine());
      await tvPlayer.play([_tvTracks.first]);
      final onTv = MusicHandoffController(
        sync: tv,
        player: tvPlayer,
        libraryTracks: () async => _tvTracks,
      );
      final payload = {
        'v': MusicQueue.jsonVersion,
        'tracks': [musicRequestToJson(_t('x', 'Unknown song'))],
        'position': 0,
        'positionMs': 1000,
      };
      final ok = await onTv.receive(
        MusicHandoffFrame(
          session: 's',
          pull: false,
          deviceId: phone.id,
          queue: jsonEncode(payload),
        ),
      );
      expect(ok, isFalse);
      expect(tvPlayer.current!.itemId, 'tv1');
      expect(
        await onTv.receive(
          const MusicHandoffFrame(
            session: 's',
            pull: false,
            deviceId: 'kphone',
            queue: '{not json',
          ),
        ),
        isFalse,
      );
    });
  });
}
