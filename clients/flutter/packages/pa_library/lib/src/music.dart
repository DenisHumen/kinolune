// SPDX-License-Identifier: MPL-2.0

// Music library for the music UI (MUS-004, docs/research/01 §4.11): tracks
// with album, credits, stream format and plays, audio playlists, and the
// grouping into albums, artists, genres, composers and folders.
//
// RustLibrary reads the tracks from the core; the v0 library and any other
// [Library] get [TracksOnlyMusic]: the plain tracks, the format guessed from
// the file extension, plays counted in memory and no playlists.

import 'dart:async';

import 'library.dart';
import 'models.dart';

/// Codecs that are lossless when the tags do not say.
const losslessCodecs = {'flac', 'alac', 'pcm', 'wav', 'ape', 'wavpack', 'wv'};

/// A music track with what the music views need.
class MusicTrack {
  final LibraryItem item;

  /// Library album id; null: tracks are grouped by album artist and title.
  final String? albumId;

  /// Album artists, main first (compilations: "Various Artists").
  final List<String> albumArtists;
  final int? albumYear;
  final bool compilation;

  /// Track artists, main first.
  final List<String> artists;
  final List<String> composers;
  final List<String> genres;
  final int? discNumber;
  final int? discTotal;

  /// `flac`, `alac`, `mp3`, `aac`, `opus`, `pcm`, `dsd`…
  final String? codec;

  /// From the tags; null: [isLossless] goes by [codec].
  final bool? lossless;
  final int? sampleRateHz;
  final int? bitDepth;
  final int? channels;
  final int? bitrateKbps;

  /// The file is on this device: playable offline ("Загружено").
  final bool local;
  final int playCount;
  final DateTime? lastPlayedAt;

  /// Front cover of the album from the Cover Art Archive (META-021); the
  /// local cover ([LibraryItem.posterUrl]) comes first.
  final String? coverUrl;

  /// MusicBrainz genres of the album, normalized (META-021).
  final List<String> albumGenres;

  /// Year of the first release of the album (MusicBrainz release group).
  final int? originalYear;

  /// MusicBrainz release-group type: `Album`, `EP`, `Single`, `Broadcast`…
  final String? albumType;

  /// Record label of the matched release.
  final String? label;

  const MusicTrack({
    required this.item,
    this.albumId,
    this.albumArtists = const [],
    this.albumYear,
    this.compilation = false,
    this.artists = const [],
    this.composers = const [],
    this.genres = const [],
    this.discNumber,
    this.discTotal,
    this.codec,
    this.lossless,
    this.sampleRateHz,
    this.bitDepth,
    this.channels,
    this.bitrateKbps,
    this.local = true,
    this.playCount = 0,
    this.lastPlayedAt,
    this.coverUrl,
    this.albumGenres = const [],
    this.originalYear,
    this.albumType,
    this.label,
  });

  /// A plain track (v0 library): the codec from the file extension, the
  /// artist and album from the item.
  factory MusicTrack.fromItem(
    LibraryItem item, {
    int playCount = 0,
    DateTime? lastPlayedAt,
  }) {
    final path = item.filePath;
    final dot = path.lastIndexOf('.');
    final ext = dot < 0 ? '' : path.substring(dot + 1).toLowerCase();
    final codec = switch (ext) {
      'flac' => 'flac',
      'mp3' => 'mp3',
      'aac' => 'aac',
      'opus' => 'opus',
      'ogg' || 'oga' => 'vorbis',
      'wav' || 'aiff' || 'aif' => 'pcm',
      'ape' => 'ape',
      'wv' => 'wavpack',
      'dsf' || 'dff' => 'dsd',
      _ => null, // .m4a may be AAC or ALAC
    };
    return MusicTrack(
      item: item,
      artists: [?item.artist],
      codec: codec,
      local: !path.contains('://') || path.startsWith('file://'),
      playCount: playCount,
      lastPlayedAt: lastPlayedAt,
    );
  }

  String get id => item.id;
  String get title => item.title;

  /// Main track artist.
  String? get artist => artists.isNotEmpty ? artists.first : item.artist;

  /// Album artist for display and grouping.
  String? get albumArtist =>
      albumArtists.isNotEmpty ? albumArtists.join(', ') : artist;

  /// More than 48 kHz or more than 16 bits.
  bool get isHiRes => (sampleRateHz ?? 0) > 48000 || (bitDepth ?? 0) > 16;

  /// FLAC, ALAC, WAV, APE, WavPack (or whatever the tags call lossless).
  bool get isLossless => lossless ?? losslessCodecs.contains(codec);

  /// The folder of the file (display path).
  String get folder {
    final i = item.filePath.lastIndexOf('/');
    return i > 0 ? item.filePath.substring(0, i) : item.filePath;
  }

  /// `FLAC 24/96`, `ALAC 16/44.1`, `DSD64`, `MP3 320`; null if unknown.
  String? get formatLabel => formatBadge(
    codec: codec,
    sampleRateHz: sampleRateHz,
    bitDepth: bitDepth,
    bitrateKbps: bitrateKbps,
    lossless: isLossless,
    extension: _extension(item.filePath),
  );

  MusicTrack withPlays(int count, DateTime? last) => MusicTrack(
    item: item,
    albumId: albumId,
    albumArtists: albumArtists,
    albumYear: albumYear,
    compilation: compilation,
    artists: artists,
    composers: composers,
    genres: genres,
    discNumber: discNumber,
    discTotal: discTotal,
    codec: codec,
    lossless: lossless,
    sampleRateHz: sampleRateHz,
    bitDepth: bitDepth,
    channels: channels,
    bitrateKbps: bitrateKbps,
    local: local,
    playCount: count,
    lastPlayedAt: last,
    coverUrl: coverUrl,
    albumGenres: albumGenres,
    originalYear: originalYear,
    albumType: albumType,
    label: label,
  );

  @override
  String toString() => 'MusicTrack($id, $title, ${formatLabel ?? '?'})';
}

String _extension(String path) {
  final dot = path.lastIndexOf('.');
  return dot < 0 ? '' : path.substring(dot + 1).toUpperCase();
}

String _khz(int hz) {
  final k = hz / 1000;
  return k == k.roundToDouble() ? '${k.round()}' : k.toStringAsFixed(1);
}

/// The short format badge of a stream: `FLAC 24/96`, `ALAC 16/44.1`,
/// `DSD64`, `MP3 320`, `OPUS`. Not translated (technical names).
String? formatBadge({
  String? codec,
  int? sampleRateHz,
  int? bitDepth,
  int? bitrateKbps,
  bool lossless = false,
  String extension = '',
}) {
  if (codec == null) return null;
  if (codec == 'dsd') {
    final rate = sampleRateHz ?? 0;
    return rate > 0 ? 'DSD${(rate / 44100).round()}' : 'DSD';
  }
  final name = switch (codec) {
    'pcm' => extension == 'AIF' || extension == 'AIFF' ? 'AIFF' : 'WAV',
    'wavpack' => 'WV',
    'vorbis' => 'OGG',
    _ => codec.toUpperCase(),
  };
  if (lossless && bitDepth != null && sampleRateHz != null) {
    return '$name $bitDepth/${_khz(sampleRateHz)}';
  }
  if (!lossless && bitrateKbps != null) return '$name $bitrateKbps';
  return name;
}

/// An audio playlist.
class MusicPlaylist {
  final String id;
  final String name;
  final int trackCount;
  final DateTime? updatedAt;

  const MusicPlaylist({
    required this.id,
    required this.name,
    this.trackCount = 0,
    this.updatedAt,
  });

  @override
  bool operator ==(Object other) =>
      other is MusicPlaylist &&
      other.id == id &&
      other.name == name &&
      other.trackCount == trackCount;

  @override
  int get hashCode => Object.hash(id, name, trackCount);
}

/// MusicBrainz data of an album artist (META-021).
class MusicArtistInfo {
  /// The artist name in the library (as tagged).
  final String name;
  final String mbid;
  final String? sortName;

  /// `Person`, `Group`, `Orchestra`, `Choir`, `Character`, `Other`.
  final String? kind;

  /// ISO 3166-1 country code.
  final String? country;
  final String? disambiguation;

  /// `1985`, `1985-03-01`.
  final String? beginDate;
  final String? endDate;

  /// Normalized genres, most votes first.
  final List<String> genres;

  const MusicArtistInfo({
    required this.name,
    required this.mbid,
    this.sortName,
    this.kind,
    this.country,
    this.disambiguation,
    this.beginDate,
    this.endDate,
    this.genres = const [],
  });

  /// Year of [beginDate].
  int? get beginYear => _year(beginDate);

  /// Year of [endDate].
  int? get endYear => _year(endDate);

  /// A group, orchestra or choir rather than one person.
  bool get isGroup => kind == 'Group' || kind == 'Orchestra' || kind == 'Choir';

  static int? _year(String? date) => date == null || date.length < 4
      ? null
      : int.tryParse(date.substring(0, 4));

  @override
  bool operator ==(Object other) =>
      other is MusicArtistInfo && other.name == name && other.mbid == mbid;

  @override
  int get hashCode => Object.hash(name, mbid);
}

/// A library with MusicBrainz data of its music (META-021: the Rust core).
abstract interface class LibraryMusicMetadata {
  /// MusicBrainz data of the matched album artists, now and after every
  /// change (the background metadata run adds them over time).
  Stream<List<MusicArtistInfo>> musicArtistInfos();
}

/// Music tracks, plays and audio playlists of a library.
abstract interface class LibraryMusic {
  /// Every present track, now and after every change.
  Stream<List<MusicTrack>> musicTracks();

  /// Counts a play of a track (it started in the player).
  Future<void> recordTrackPlay(String trackId);

  /// Whether this library keeps audio playlists.
  bool get hasMusicPlaylists;

  /// Audio playlists, most recently changed first, now and after every
  /// change.
  Stream<List<MusicPlaylist>> musicPlaylists();
  Future<MusicPlaylist> createMusicPlaylist(String name);
  Future<void> renameMusicPlaylist(String id, String name);
  Future<void> deleteMusicPlaylist(String id);

  /// Tracks of a playlist in order.
  Future<List<LibraryItem>> musicPlaylistItems(String id);

  /// `false` if the track already was in the playlist.
  Future<bool> addToMusicPlaylist(String id, String trackId);
  Future<bool> removeFromMusicPlaylist(String id, String trackId);
}

/// [LibraryMusic] of a library without one (the v0 library): its tracks as
/// [MusicTrack.fromItem], plays counted in memory for this run, no
/// playlists.
class TracksOnlyMusic implements LibraryMusic {
  TracksOnlyMusic(this.library, {DateTime Function()? clock})
    : _clock = clock ?? DateTime.now;

  final Library library;
  final DateTime Function() _clock;
  final Map<String, (int, DateTime)> _plays = {};
  final _played = StreamController<void>.broadcast();

  @override
  Stream<List<MusicTrack>> musicTracks() => Stream.multi((c) {
    List<LibraryItem>? last;
    void emit() {
      final items = last;
      if (items == null) return;
      c.add([
        for (final i in items)
          MusicTrack.fromItem(
            i,
            playCount: _plays[i.id]?.$1 ?? 0,
            lastPlayedAt: _plays[i.id]?.$2,
          ),
      ]);
    }

    final subs = [
      library.tracks().listen((items) {
        last = items;
        emit();
      }, onError: c.addError),
      _played.stream.listen((_) => emit()),
    ];
    c.onCancel = () async {
      for (final s in subs) {
        await s.cancel();
      }
    };
  });

  @override
  Future<void> recordTrackPlay(String trackId) async {
    final (n, _) = _plays[trackId] ?? (0, _clock());
    _plays[trackId] = (n + 1, _clock());
    _played.add(null);
  }

  @override
  bool get hasMusicPlaylists => false;

  @override
  Stream<List<MusicPlaylist>> musicPlaylists() => Stream.value(const []);

  @override
  Future<MusicPlaylist> createMusicPlaylist(String name) =>
      throw UnsupportedError('no playlists in this library');

  @override
  Future<void> renameMusicPlaylist(String id, String name) =>
      throw UnsupportedError('no playlists in this library');

  @override
  Future<void> deleteMusicPlaylist(String id) =>
      throw UnsupportedError('no playlists in this library');

  @override
  Future<List<LibraryItem>> musicPlaylistItems(String id) async => const [];

  @override
  Future<bool> addToMusicPlaylist(String id, String trackId) =>
      throw UnsupportedError('no playlists in this library');

  @override
  Future<bool> removeFromMusicPlaylist(String id, String trackId) async =>
      false;
}

final _fallbacks = Expando<TracksOnlyMusic>();

/// The music of [library]: itself if it is a [LibraryMusic], else one
/// [TracksOnlyMusic] per library (so plays counted in memory last).
LibraryMusic libraryMusic(Library library) => library is LibraryMusic
    ? library as LibraryMusic
    : (_fallbacks[library] ??= TracksOnlyMusic(library));
