// SPDX-License-Identifier: MPL-2.0

// Blu-ray and DVD folders and images (PLAY-017, ADR-0140). The core reads
// the disc — in place, an `.iso` is never unpacked — and tells what its
// main title plays: the files of each angle, the chapters, the languages
// of the audio and subtitle streams; the other titles longer than about
// two minutes are extras; a protected disc is not played.

import 'dart:convert';

import 'item_details.dart';
import 'models.dart';

/// An audio or subtitle stream of a disc title, in the order of the disc.
class DiscStream {
  const DiscStream({
    required this.subtitle,
    required this.id,
    required this.codec,
    required this.language,
  });

  /// A subtitle stream (PGS, VobSub); else audio.
  final bool subtitle;

  /// PID of a Blu-ray stream (`0x1100`), substream of a DVD one (`0x80`):
  /// what libmpv reports as the track's `src-id`.
  final int id;
  final String codec;

  /// ISO 639-2 (`eng`) or 639-1 (`en`); empty if the disc does not say.
  final String language;
}

/// A title of a disc: the main film or an extra.
class DiscTitle {
  const DiscTitle({
    required this.id,
    required this.durationMs,
    required this.angles,
    this.chaptersMs = const [],
    this.streams = const [],
    this.uhd = false,
    this.is3d = false,
  });

  /// Playlist (`00800`) or title set (`VTS_01`).
  final String id;
  final int durationMs;

  /// For each angle, the files to play back to back: local paths or
  /// loopback stream URLs. At least one angle.
  final List<List<String>> angles;
  final List<int> chaptersMs;
  final List<DiscStream> streams;
  final bool uhd;
  final bool is3d;

  /// One URI per angle for the player ([discUri]).
  List<String> get angleUris => [for (final a in angles) discUri(a)];
}

/// What a disc item plays.
class DiscPlayback {
  const DiscPlayback({
    required this.bluRay,
    this.protection,
    this.volumeId = '',
    this.main,
    this.extras = const [],
  });

  /// Blu-ray; else DVD.
  final bool bluRay;

  /// `aacs`, `bdplus` or `css`: the disc is protected and is not played.
  final String? protection;
  final String volumeId;

  /// Null: no playable title.
  final DiscTitle? main;
  final List<DiscTitle> extras;

  /// The extras as cards of «Дополнительно»: one per title, playing its
  /// first angle.
  List<MediaExtra> get extraCards => [
    for (final e in extras)
      if (e.angles.isNotEmpty && e.angles.first.isNotEmpty)
        MediaExtra(
          title: e.id,
          kind: MediaExtraKind.discTitle,
          filePath: e.angles.first.first,
          streamUri: discUri(e.angles.first),
          durationMs: e.durationMs,
        ),
  ];
}

/// One URI that plays [files] back to back: the file itself, or an mpv
/// EDL (`edl://%<bytes>%<file>;…`) like the player's `partsUri` (LIB-020).
String discUri(List<String> files) {
  if (files.length == 1) return files.single;
  return 'edl://${files.map((f) => '%${utf8.encode(f).length}%$f').join(';')}';
}

/// A library that reads Blu-ray and DVD folders and images.
abstract interface class LibraryDisc {
  /// What [item]'s file plays if it is on a disc (a disc folder's first
  /// clip, an image); null for any other file.
  Future<DiscPlayback?> discPlayback(LibraryItem item);
}
