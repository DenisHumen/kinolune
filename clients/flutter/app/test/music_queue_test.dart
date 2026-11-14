// SPDX-License-Identifier: MPL-2.0
//
// MUS-003 in the app: the queue survives an app restart (per profile),
// Play Next / Add to Queue from the track menu, "Save as playlist", a
// video pauses the music and offers it back, smart playlists and M3U.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_profiles.dart';
import 'package:kinolune/features/music/m3u.dart';
import 'package:kinolune/features/music/music_queue_store.dart';
import 'package:kinolune/features/music/playlist_actions.dart';
import 'package:kinolune/features/music/smart_playlists.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/music/smart_playlist_pages.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

Future<List<MusicTrack>> _tracks(AppServices s) =>
    libraryMusic(s.library).musicTracks().first;

MusicTrack _track(
  String id,
  String path, {
  List<String> genres = const [],
  int? year,
  int plays = 0,
  String? codec,
  int? bits,
  int? rate,
}) => MusicTrack(
  item: LibraryItem(
    id: id,
    kind: ItemKind.track,
    title: 'T $id',
    filePath: path,
    durationMs: 61000,
    artist: 'A',
  ),
  genres: genres,
  albumYear: year,
  playCount: plays,
  codec: codec,
  bitDepth: bits,
  sampleRateHz: rate,
);

void main() {
  group('queue across restarts', () {
    testWidgets('the queue survives an app restart, paused where it was', (
      t,
    ) async {
      final prefs = MemoryPreferences();
      final engine = FakePlayerEngine();
      final s1 = await demoServices(
        preferences: prefs,
        engineFactory: ({bool video = true}) => engine,
      );
      await s1.musicQueue.ready;
      final tracks = await _tracks(s1);
      expect(tracks.length, greaterThan(4));
      final reqs = [for (final x in tracks.take(4)) musicRequest(x)];
      await s1.music.play(
        reqs,
        index: 1,
        source: const QueueSource('Nocturne Avenue'),
      );
      await s1.music.addToQueue([musicRequest(tracks[4])]);
      await s1.music.setRepeat(QueueRepeat.all);
      engine.advance(const Duration(seconds: 37));
      s1.music.pause();
      // Saved by itself shortly after the change.
      await t.pump(const Duration(milliseconds: 600));
      final key = Preferences.musicQueue('default');
      expect(prefs.values[key], isNotNull);
      final saved = jsonDecode(prefs.values[key]!) as Map<String, Object?>;
      expect(saved['positionMs'], 37000);

      // "Restart": new services, new engine, same preferences.
      final engine2 = FakePlayerEngine();
      final s2 = await demoServices(
        preferences: prefs,
        engineFactory: ({bool video = true}) => engine2,
      );
      await s2.musicQueue.ready;
      final m = s2.music;
      expect(m.current?.itemId, tracks[1].id);
      expect(m.playing, isFalse);
      expect(m.position, const Duration(seconds: 37));
      expect(m.source?.label, 'Nocturne Avenue');
      expect(m.repeat, QueueRepeat.all);
      expect(m.queue.queuedCount, 1);
      expect([for (final r in m.queue.ordered) r.itemId], [
        for (final i in [0, 1, 4, 2, 3]) tracks[i].id,
      ]);
      expect(m.recent.first.itemId, tracks[1].id);
      expect(engine2.calls.where((c) => c.startsWith('open')), isEmpty);

      m.resume();
      await t.pump();
      expect(engine2.calls, contains('open:5:1:37000:true'));
    });

    testWidgets('each profile keeps its own queue', (t) async {
      final prefs = MemoryPreferences()
        ..values[Preferences.profilesPickerOnStart] = 'false';
      final profiles = FakeProfiles.demo();
      final s = await demoServices(preferences: prefs, profiles: profiles);
      await s.musicQueue.ready;
      final tracks = await _tracks(s);
      await s.music.play([musicRequest(tracks[0]), musicRequest(tracks[1])]);
      await s.musicQueue.save();

      final sam = (await profiles.profiles()).firstWhere(
        (p) => p.syncId == 'p-sam',
      );
      await s.profileControl.switchTo(sam);
      await s.musicQueue.ready;
      expect(s.music.hasQueue, isFalse, reason: "Sam's queue is empty");
      await s.music.play([musicRequest(tracks[2])]);
      await s.musicQueue.save();
      expect(prefs.values[Preferences.musicQueue('p-sam')], isNotNull);

      final alex = (await profiles.profiles()).firstWhere(
        (p) => p.syncId == 'default',
      );
      await s.profileControl.switchTo(alex, pin: '1234');
      await s.musicQueue.ready;
      expect(s.music.current?.itemId, tracks[0].id);
      expect(s.music.queue.length, 2);
    });

    test('tracks gone from the library drop out on restore', () async {
      final prefs = MemoryPreferences();
      final player = MusicPlayerController(
        engineFactory: ({bool video = true}) => FakePlayerEngine(),
      );
      final all = [
        for (var i = 0; i < 3; i++)
          PlayRequest(uri: '/old/$i.flac', itemId: 'id$i', title: 'T$i'),
      ];
      await player.play(all);
      final store = MusicQueueStore(
        player: player,
        preferences: prefs,
        profileId: () async => 'p',
        libraryTracks: () async => {
          'id0': const PlayRequest(uri: '/new/0.flac', itemId: 'id0', title: 'T0'),
          'id2': const PlayRequest(uri: '/new/2.flac', itemId: 'id2', title: 'T2'),
        },
      );
      await store.load();
      expect(player.queue.length, 3, reason: 'nothing saved: left alone');
      await store.save();
      final again = MusicPlayerController(
        engineFactory: ({bool video = true}) => FakePlayerEngine(),
      );
      final store2 = MusicQueueStore(
        player: again,
        preferences: prefs,
        profileId: () async => 'p',
        libraryTracks: () async => {
          'id0': const PlayRequest(uri: '/new/0.flac', itemId: 'id0', title: 'T0'),
          'id2': const PlayRequest(uri: '/new/2.flac', itemId: 'id2', title: 'T2'),
        },
      );
      await store2.load();
      expect([for (final r in again.queue.ordered) r.uri], [
        '/new/0.flac',
        '/new/2.flac',
      ]);
      store.dispose();
      store2.dispose();
    });
  });

  group('music UI', () {
    setUpAll(loadAppFonts);

    testWidgets('track menu: Play Next and Add to Queue', (t) async {
      setSurface(t, const Size(1280, 800));
      final s = await demoServices();
      await t.pumpWidget(testApp(s, section: Section.music));
      await settle(t);
      await t.tap(find.text('Nocturne Avenue').first);
      await settle(t);
      final tracks = await _tracks(s);
      final album = tracks
          .where((x) => x.item.album == 'Nocturne Avenue')
          .toList();
      // Start the album, then queue two of its tracks.
      await t.tap(find.text('Play').first);
      await settle(t, 4);
      expect(s.music.source?.label, 'Nocturne Avenue');

      Future<void> menu(MusicTrack x, String item) async {
        final row = find.byKey(ValueKey('track-${x.id}')).first;
        await t.ensureVisible(row);
        await settle(t, 2);
        await t.longPress(row);
        await settle(t, 4);
        await t.tap(find.byKey(ValueKey(item)));
        await settle(t, 4);
      }

      await menu(album.last, 'track-menu-queue');
      expect(s.music.queue.upNext[0].itemId, album.last.id);
      expect(find.text('Added to the queue'), findsOneWidget);
      await menu(album[2], 'track-menu-next');
      expect(s.music.queue.upNext[0].itemId, album[2].id);
      expect(s.music.queue.upNext[1].itemId, album.last.id);
      expect(s.music.queue.queuedCount, 2);
      s.music.pause();
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('Now Playing: save the queue as a playlist', (t) async {
      setSurface(t, const Size(1280, 800));
      final s = await demoServices();
      final tracks = await _tracks(s);
      await s.music.play(
        [for (final x in tracks.take(3)) musicRequest(x)],
        source: const QueueSource('Mix'),
      );
      await t.pumpWidget(testApp(s, section: Section.music));
      await settle(t);
      openNowPlaying(t.element(find.byType(AppShell)));
      await settle(t);
      expect(find.text('Up next from: Mix'), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('np-save')));
      await settle(t, 4);
      await t.enterText(find.byKey(const ValueKey('playlist-name')), 'Saved');
      await t.tap(find.byKey(const ValueKey('playlist-name-ok')));
      await settle(t, 4);
      final lists = await libraryMusic(s.library).musicPlaylists().first;
      final saved = lists.firstWhere((p) => p.name == 'Saved');
      expect(saved.trackCount, 3);
      final items = await libraryMusic(s.library).musicPlaylistItems(saved.id);
      expect([for (final i in items) i.id], [
        for (final x in tracks.take(3)) x.id,
      ]);
      s.music.pause();
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('a video pauses the music and offers to resume it', (t) async {
      setSurface(t, const Size(1280, 800));
      final s = await demoServices();
      final tracks = await _tracks(s);
      await s.music.play([musicRequest(tracks.first)]);
      expect(s.music.playing, isTrue);
      await t.pumpWidget(testApp(s));
      await settle(t);
      final context = t.element(find.byType(AppShell));
      expect(s.music.pauseForVideo(), isTrue);
      expect(s.music.playing, isFalse);
      // The video closed.
      offerMusicResume(
        s,
        ScaffoldMessenger.of(context),
        lookupAppLocalizations(const Locale('en')),
      );
      await settle(t, 4);
      expect(find.text('Music paused for the video'), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('music-resume')));
      await settle(t, 4);
      expect(s.music.playing, isTrue);
      s.music.pause();
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('a smart playlist from the playlists section', (t) async {
      setSurface(t, const Size(1280, 800));
      final s = await demoServices();
      await t.pumpWidget(testApp(s, section: Section.music));
      await settle(t);
      await t.tap(find.byKey(const ValueKey('music-tab-library')));
      await settle(t, 4);
      await t.tap(find.byKey(const ValueKey('music-section-playlists')));
      await settle(t, 4);
      await t.tap(find.byKey(const ValueKey('music-smart-new')));
      await settle(t, 4);
      await t.enterText(find.byKey(const ValueKey('smart-name')), 'Lossless');
      await t.tap(find.byKey(const ValueKey('smart-rule-0-field')));
      await settle(t, 4);
      await t.tap(find.text('Format').last);
      await settle(t, 4);
      await t.tap(find.byKey(const ValueKey('smart-save')));
      await settle(t, 4);
      final saved = s.smartPlaylists.playlists.value.single;
      expect(saved.name, 'Lossless');
      expect(saved.rules.single, const SmartRule(
        SmartField.format,
        SmartOp.equals,
        smartFormatLossless,
      ));
      final tracks = await _tracks(s);
      final expected = tracks.where((x) => x.isLossless).length;
      expect(find.byKey(ValueKey('music-smart-${saved.id}')), findsOneWidget);
      await t.tap(find.byKey(ValueKey('music-smart-${saved.id}')));
      await settle(t, 4);
      expect(find.byType(SmartPlaylistScreen), findsOneWidget);
      final en = lookupAppLocalizations(const Locale('en'));
      expect(find.textContaining(en.musicTrackCount(expected)), findsWidgets);
      // Kept for the profile.
      expect(
        await s.preferences.read(Preferences.musicSmartPlaylists('default')),
        contains('Lossless'),
      );
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });

  group('smart playlists', () {
    final lib = [
      _track('1', '/m/1.flac', genres: ['Jazz'], year: 1959, plays: 12,
          codec: 'flac', bits: 24, rate: 96000),
      _track('2', '/m/2.mp3', genres: ['Acid Jazz'], year: 1994, plays: 3,
          codec: 'mp3'),
      _track('3', '/m/3.flac', genres: ['Rock'], year: 1971, plays: 0,
          codec: 'flac', bits: 16, rate: 44100),
      _track('4', '/m/4.opus', genres: ['Jazz'], year: 2020, plays: 7,
          codec: 'opus'),
    ];
    List<String> ids(SmartPlaylist p) => [for (final t in p.evaluate(lib)) t.id];

    test('rules over genre, year, plays and format', () {
      const jazz = SmartPlaylist(
        id: 'a',
        name: 'Jazz',
        rules: [SmartRule(SmartField.genre, SmartOp.contains, 'jazz')],
      );
      expect(ids(jazz), ['1', '2', '4']);
      expect(
        ids(jazz.copyWith(
          rules: const [SmartRule(SmartField.genre, SmartOp.equals, 'jazz')],
        )),
        ['1', '4'],
      );
      const old = SmartPlaylist(
        id: 'b',
        name: 'Old lossless',
        rules: [
          SmartRule(SmartField.year, SmartOp.atMost, '1980'),
          SmartRule(SmartField.format, SmartOp.equals, smartFormatLossless),
        ],
      );
      expect(ids(old), ['1', '3']);
      expect(
        ids(old.copyWith(rules: const [
          SmartRule(SmartField.format, SmartOp.equals, smartFormatHiRes),
        ])),
        ['1'],
      );
      expect(
        ids(old.copyWith(rules: const [
          SmartRule(SmartField.format, SmartOp.notEquals, 'flac'),
        ])),
        ['2', '4'],
      );
      const any = SmartPlaylist(
        id: 'c',
        name: 'Any',
        matchAll: false,
        sort: SmartSort.mostPlayed,
        limit: 2,
        rules: [
          SmartRule(SmartField.plays, SmartOp.atLeast, '5'),
          SmartRule(SmartField.genre, SmartOp.equals, 'rock'),
        ],
      );
      expect(ids(any), ['1', '4'], reason: 'most played first, two at most');
      expect(ids(any.copyWith(limit: () => null)), ['1', '4', '3']);
      expect(
        ids(any.copyWith(sort: SmartSort.year, limit: () => null)),
        ['1', '3', '4'],
      );
    });

    test('stored as rules (JSON) per profile', () async {
      final prefs = MemoryPreferences();
      var profile = 'one';
      final store = SmartPlaylistStore(prefs, () async => profile);
      await store.load();
      const p = SmartPlaylist(
        id: 'x',
        name: 'Recent',
        sort: SmartSort.recentlyPlayed,
        limit: 25,
        rules: [SmartRule(SmartField.year, SmartOp.atLeast, '2000')],
      );
      await store.save(p);
      final raw = prefs.values[Preferences.musicSmartPlaylists('one')]!;
      expect(jsonDecode(raw), [p.toJson()]);
      final again = SmartPlaylistStore(prefs, () async => 'one');
      await again.load();
      expect(again.playlists.value.single, p);

      profile = 'two';
      await store.load();
      expect(store.playlists.value, isEmpty);
      profile = 'one';
      await store.delete('x');
      expect(store.playlists.value, isEmpty);
      expect(SmartPlaylist.fromJson({'id': 1}), isNull);
      expect(
        SmartRule.fromJson({'field': 'year', 'op': 'contains', 'value': '1'}),
        isNull,
        reason: 'an operator the field has no use for',
      );
    });
  });

  group('M3U', () {
    final lib = [
      _track('a', '/home/u/Music/Artist/Album/01 One.flac'),
      _track('b', '/home/u/Music/Artist/Album/02 Two.flac'),
      _track('c', 'smb://nas/music/Other/03 Three.mp3'),
      _track('d', '/home/u/Music/X/01 One.flac'),
    ];

    test('export writes relative paths next to the music', () {
      final text = writeM3u(
        lib.take(3).toList(),
        playlistPath: '/home/u/Music/Playlists/mix.m3u8',
      );
      expect(const LineSplitter().convert(text), [
        '#EXTM3U',
        '#EXTINF:61,A - T a',
        '../Artist/Album/01 One.flac',
        '#EXTINF:61,A - T b',
        '../Artist/Album/02 Two.flac',
        '#EXTINF:61,A - T c',
        'smb://nas/music/Other/03 Three.mp3',
      ]);
      expect(relativePath('/a/b/c.flac', '/a/b'), 'c.flac');
      expect(relativePath('C:\\M\\x.flac', 'D:\\P'), isNull);
    });

    test('import resolves relative, absolute, URI and foreign paths', () {
      const text =
          '\ufeff#EXTM3U\n'
          '#EXTINF:61,A - T b\n'
          'Album/02 Two.flac\n'
          '\n'
          '# a comment\n'
          'file:///home/u/Music/Artist/Album/01%20One.flac\n'
          'smb://NAS/music/Other/03 Three.mp3\n'
          'D:\\Backup\\Music\\Artist\\Album\\01 One.flac\n'
          '../missing.flac\n';
      final entries = parseM3u(decodeM3u(utf8.encode(text)));
      expect(entries.first.title, 'A - T b');
      expect(entries.first.durationSeconds, 61);
      final m = resolveM3u(
        entries,
        lib,
        playlistPath: '/home/u/Music/Artist/list.m3u',
      );
      expect([for (final t in m.tracks) t.id], ['b', 'a', 'c', 'a']);
      expect(m.missing, ['../missing.flac']);
    });

    test('a bare file name shared by two tracks is ambiguous', () {
      final m = resolveM3u([const M3uEntry('01 One.flac')], lib);
      expect(m.tracks, isEmpty);
      expect(m.missing, ['01 One.flac']);
    });

    test('Latin-1 .m3u and the playlist name', () {
      expect(decodeM3u(latin1.encode('Café.flac')), 'Café.flac');
      expect(m3uPlaylistName('/x/Road Trip.m3u8'), 'Road Trip');
    });

    test('import makes a playlist of the found tracks', () async {
      final library = FakeLibrary.demo(scanStepDelay: Duration.zero);
      final music = libraryMusic(library);
      final tracks = await music.musicTracks().first;
      final text = writeM3u(tracks.take(3).toList());
      final r = await importM3uPlaylist(
        music,
        name: 'Imported',
        text: '$text\n/nowhere/x.flac\n',
      );
      expect(r, isNotNull);
      expect(r!.added, 3);
      expect(r.missing, ['/nowhere/x.flac']);
      final items = await music.musicPlaylistItems(r.playlist.id);
      expect([for (final i in items) i.id], [
        for (final t in tracks.take(3)) t.id,
      ]);
      expect(
        await importM3uPlaylist(music, name: 'None', text: '/nowhere.flac'),
        isNull,
      );
    });
  });
}
