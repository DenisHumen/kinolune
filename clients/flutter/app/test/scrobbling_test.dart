// SPDX-License-Identifier: MPL-2.0
//
// MUS-015 in the app: the scrobbler follows the music player (track
// changes, the playhead every 5 s and on pause), stays silent until an
// account is connected, keeps the accounts per profile in the keychain and
// runs the Last.fm and ListenBrainz sign-ins. The rules and the offline
// queue themselves are tested in the core (`pa-scrobble`).

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/music/scrobbling.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

class _Backend implements ScrobbleBackend {
  final calls = <String>[];
  ({String? lastfm, String? key, String? lb})? config;
  String? refuse;

  @override
  bool hasProjectLastfmKey = false;

  @override
  Future<ScrobbleStatus> configure({
    String? lastfmSessionKey,
    String? lastfmApiKey,
    String? lastfmSecret,
    String? listenbrainzToken,
  }) async {
    config = (lastfm: lastfmSessionKey, key: lastfmApiKey, lb: listenbrainzToken);
    return const ScrobbleStatus(lastfmPending: 2);
  }

  @override
  Future<void> trackStarted(String id, ScrobbleTrack t, int positionMs) async =>
      calls.add('start:$id:${t.artist}/${t.title}:$positionMs');

  @override
  Future<void> progress(int positionMs, bool playing) async =>
      calls.add('progress:$positionMs:$playing');

  @override
  Future<void> stopped() async => calls.add('stopped');

  @override
  Future<ScrobbleStatus> flush() async {
    calls.add('flush');
    return const ScrobbleStatus();
  }

  @override
  Future<LastFmAuthStart> lastfmBegin({String? apiKey, String? secret}) async {
    if (apiKey == null && !hasProjectLastfmKey) {
      throw const ScrobbleException('no_key');
    }
    return (token: 'tok', url: 'https://www.last.fm/api/auth/?token=tok');
  }

  @override
  Future<({String name, String sessionKey})> lastfmFinish(
    String token, {
    String? apiKey,
    String? secret,
  }) async {
    if (refuse != null) throw ScrobbleException(refuse!);
    return (name: 'listener', sessionKey: 'sk-$token');
  }

  @override
  Future<String> listenbrainzUser(String token) async {
    if (token != 'good') throw const ScrobbleException('refused');
    return 'lb-user';
  }
}

PlayRequest _t(String id, {String? artist = 'Band'}) => PlayRequest(
  uri: '/m/$id.flac',
  itemId: id,
  title: 'Song $id',
  artist: artist,
  album: 'Album',
  durationMs: 200000,
);

void main() {
  late FakePlayerEngine engine;
  late MusicPlayerController player;
  late _Backend backend;
  late MemorySecureStore store;
  late DateTime now;
  var profile = 'kid';

  MusicScrobbleController controller() => MusicScrobbleController(
    backend: backend,
    player: player,
    secureStore: store,
    profileId: () async => profile,
    clock: () => now,
  );

  setUp(() {
    engine = FakePlayerEngine();
    player = MusicPlayerController(engineFactory: ({bool video = true}) => engine);
    backend = _Backend();
    store = MemorySecureStore();
    now = DateTime.utc(2026, 11, 24, 12);
    profile = 'kid';
  });

  test('silent until an account is connected', () async {
    final s = controller();
    addTearDown(s.dispose);
    await s.load();
    expect(s.enabled, isFalse);
    expect(s.needsOwnLastfmKey, isTrue);
    await player.play([_t('1')]);
    await pumpEventQueue();
    expect(backend.calls, isEmpty);
  });

  test('follows tracks, the playhead every 5 s and pauses', () async {
    final s = controller();
    addTearDown(s.dispose);
    await s.load();
    await s.connectListenbrainz(' good ');
    expect(backend.config?.lb, 'good');
    expect(s.accounts.listenbrainzUser, 'lb-user');

    await player.play([_t('1'), _t('2'), _t('3', artist: null)]);
    await pumpEventQueue();
    expect(backend.calls, ['start:1:Band/Song 1:0']);

    for (var i = 0; i < 6; i++) {
      now = now.add(const Duration(seconds: 1));
      engine.advance(const Duration(seconds: 1));
    }
    await pumpEventQueue();
    expect(backend.calls.where((c) => c.startsWith('progress')), [
      'progress:5000:true',
    ]);

    player.pause();
    await pumpEventQueue();
    expect(backend.calls.last, 'progress:6000:false');

    backend.calls.clear();
    player.next();
    await pumpEventQueue();
    expect(backend.calls, isEmpty, reason: 'paused: starts when it plays');
    player.resume();
    await pumpEventQueue();
    expect(backend.calls.first, startsWith('start:2:'));
    player.next();
    await pumpEventQueue();
    expect(
      backend.calls.last,
      'stopped',
      reason: 'a track without an artist is not scrobbled',
    );
  });

  test('accounts are kept per profile in the keychain', () async {
    final s = controller();
    addTearDown(s.dispose);
    await s.load();
    final start = await s.beginLastfm().then<Object?>((v) => v, onError: (e) => e);
    expect(start, isA<ScrobbleException>(), reason: 'no key in this build');
    await s.setLastfmKey(' my-key ', 'my-secret');
    final auth = await s.beginLastfm();
    expect(auth.url, contains('token=tok'));
    await s.finishLastfm(auth.token);
    expect(s.accounts.lastfmName, 'listener');
    expect(backend.config, (lastfm: 'sk-tok', key: 'my-key', lb: null));
    expect(s.status.lastfmPending, 2);
    final saved = await store.read(scrobbleAccountsKey('kid'));
    expect(saved, contains('sk-tok'));
    expect(s.accounts.toString(), isNot(contains('sk-tok')));

    profile = 'adult';
    await s.switchProfile();
    expect(s.accounts.lastfmConnected, isFalse);
    expect(backend.config, (lastfm: null, key: null, lb: null));

    profile = 'kid';
    await s.switchProfile();
    expect(s.accounts.lastfmName, 'listener');

    await s.disconnectLastfm();
    expect(backend.config?.lastfm, isNull);
    expect(
      await store.read(scrobbleAccountsKey('kid')),
      isNot(contains('sk-tok')),
    );

    backend.refuse = 'refused';
    await expectLater(
      s.finishLastfm('not-approved'),
      throwsA(isA<ScrobbleException>()),
    );
    await expectLater(
      s.connectListenbrainz('bad'),
      throwsA(isA<ScrobbleException>()),
    );
    expect(s.accounts.listenbrainzConnected, isFalse);
  });

  test('without the core nothing runs', () async {
    final s = MusicScrobbleController(
      backend: null,
      player: player,
      secureStore: store,
      profileId: () async => null,
    );
    addTearDown(s.dispose);
    await s.load();
    expect(s.available, isFalse);
    await s.flush();
  });
}
