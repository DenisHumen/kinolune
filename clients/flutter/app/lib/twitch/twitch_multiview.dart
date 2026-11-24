// SPDX-License-Identifier: MPL-2.0
//
// Multiview (TWI-022, research 06 §6.4): 2–4 live streams at once on a
// desktop, a TV or a tablet. This file is the part without a player:
//
// - the layouts (1+1, 2×2, one big + three small) as slot rectangles;
// - which tile is heard: one at a time, the others muted; in "one big"
//   the heard tile is the big one;
// - the quality cap of a tile by how many share the screen (four tiles
//   720p, the big one 1080p, the small ones 480p) to save bandwidth and
//   CPU;
// - moving between tiles with the arrows of a remote or a keyboard.
//
// The player side ([MultiviewStream]) is engine-agnostic: the screen gets
// native tiles (TWI-014, the signed config allows it) or official embeds
// (TWI-005) where the platform's WebView can show several at once.

import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import 'twitch_embed.dart' show twitchEmbedParent;

/// At most this many streams at once.
const kMultiviewMax = 4;

/// How the tiles share the screen.
enum MultiviewLayout {
  /// 1+1: two tiles side by side (stacked on a portrait tablet); one tile
  /// fills the screen.
  pair,

  /// 2×2.
  grid,

  /// One big tile (the heard one) and up to three small ones beside it.
  focus,
}

/// The layouts that make sense for [count] tiles.
List<MultiviewLayout> multiviewLayoutsFor(int count) => count <= 2
    ? const [MultiviewLayout.pair]
    : const [MultiviewLayout.grid, MultiviewLayout.focus];

/// The layout a screen of [count] tiles starts with.
MultiviewLayout multiviewDefaultLayout(int count) =>
    count <= 2 ? MultiviewLayout.pair : MultiviewLayout.grid;

/// Slots on the screen: tiles plus free places ("add a stream").
int multiviewSlotCount(MultiviewLayout layout, int count) => switch (layout) {
  MultiviewLayout.pair => math.max(1, math.min(count, 2)),
  MultiviewLayout.grid || MultiviewLayout.focus => kMultiviewMax,
};

/// Which tile sits in each slot: in [MultiviewLayout.focus] the heard tile
/// ([audio]) takes the big slot 0, the rest keep their order; null is a
/// free slot.
List<int?> multiviewSlotTiles(MultiviewLayout layout, int count, int audio) {
  final slots = multiviewSlotCount(layout, count);
  final order = <int>[
    if (layout == MultiviewLayout.focus && audio >= 0 && audio < count) audio,
    for (var i = 0; i < count; i++)
      if (layout != MultiviewLayout.focus || i != audio) i,
  ];
  return [for (var s = 0; s < slots; s++) s < order.length ? order[s] : null];
}

/// The slot rectangles of [layout] for [count] tiles in [area], [gap]
/// between them.
List<Rect> multiviewSlots(
  MultiviewLayout layout,
  int count,
  Size area, {
  double gap = 4,
}) {
  final w = area.width;
  final h = area.height;
  switch (layout) {
    case MultiviewLayout.pair:
      if (count <= 1) return [Offset.zero & area];
      if (w >= h) {
        final half = (w - gap) / 2;
        return [
          Rect.fromLTWH(0, 0, half, h),
          Rect.fromLTWH(half + gap, 0, half, h),
        ];
      }
      final half = (h - gap) / 2;
      return [
        Rect.fromLTWH(0, 0, w, half),
        Rect.fromLTWH(0, half + gap, w, half),
      ];
    case MultiviewLayout.grid:
      final cw = (w - gap) / 2;
      final ch = (h - gap) / 2;
      return [
        Rect.fromLTWH(0, 0, cw, ch),
        Rect.fromLTWH(cw + gap, 0, cw, ch),
        Rect.fromLTWH(0, ch + gap, cw, ch),
        Rect.fromLTWH(cw + gap, ch + gap, cw, ch),
      ];
    case MultiviewLayout.focus:
      // The big tile three quarters wide, the small ones in a column.
      final bw = (w - gap) * 3 / 4;
      final sw = w - gap - bw;
      final sh = (h - 2 * gap) / 3;
      return [
        Rect.fromLTWH(0, 0, bw, h),
        for (var i = 0; i < 3; i++)
          Rect.fromLTWH(bw + gap, i * (sh + gap), sw, sh),
      ];
  }
}

/// The quality cap of the tile in [slot] (a [pickQuality] preference): one
/// tile `1080`; two or four `720`; in "one big" the big tile `1080`, the
/// small ones `480`.
String multiviewQuality(MultiviewLayout layout, int count, int slot) {
  if (count <= 1) return '1080';
  return switch (layout) {
    MultiviewLayout.pair || MultiviewLayout.grid => '720',
    MultiviewLayout.focus => slot == 0 ? '1080' : '480',
  };
}

/// The slot an arrow in [direction] moves to from [from], or null at the
/// edge: the nearest slot that lies that way, preferring one in line.
int? multiviewNeighbor(List<Rect> slots, int from, AxisDirection direction) {
  if (from < 0 || from >= slots.length) return null;
  final a = slots[from].center;
  int? best;
  var bestScore = double.infinity;
  for (var i = 0; i < slots.length; i++) {
    if (i == from) continue;
    final b = slots[i].center;
    final dx = b.dx - a.dx;
    final dy = b.dy - a.dy;
    final (along, across) = switch (direction) {
      AxisDirection.right => (dx, dy.abs()),
      AxisDirection.left => (-dx, dy.abs()),
      AxisDirection.down => (dy, dx.abs()),
      AxisDirection.up => (-dy, dx.abs()),
    };
    if (along <= 1) continue;
    final score = along + 2 * across;
    if (score < bestScore) {
      bestScore = score;
      best = i;
    }
  }
  return best;
}

/// The Twitch embed must be at least 400×300 and uncovered: every slot of
/// [slots] is big enough for it.
bool multiviewFitsEmbed(Iterable<Rect> slots) =>
    slots.every((r) => r.width >= 400 && r.height >= 300);

/// The streams on the screen, the heard one and the layout.
class MultiviewController extends ChangeNotifier {
  MultiviewController(Iterable<String> logins, {this.max = kMultiviewMax})
    : _logins = [] {
    for (final l in logins) {
      if (_logins.length < max && !_logins.contains(l)) _logins.add(l);
    }
    _layout = multiviewDefaultLayout(_logins.length);
  }

  /// 4 on a desktop or a TV, 2 on a tablet.
  final int max;
  final List<String> _logins;
  int _audio = 0;
  late MultiviewLayout _layout;

  List<String> get logins => List.unmodifiable(_logins);
  int get count => _logins.length;
  bool get canAdd => _logins.length < max;
  MultiviewLayout get layout => _layout;

  /// The heard tile (in "one big" also the big one); -1 when empty.
  int get audio => _logins.isEmpty ? -1 : _audio;
  String? get audioLogin => _logins.isEmpty ? null : _logins[_audio];

  /// Which tile is in each slot ([multiviewSlotTiles]).
  List<int?> get slotTiles => multiviewSlotTiles(_layout, count, audio);

  /// The slot of the tile [index].
  int slotOf(int index) => slotTiles.indexOf(index);

  /// The quality cap of [login] ([multiviewQuality]).
  String qualityOf(String login) {
    final i = _logins.indexOf(login);
    return multiviewQuality(_layout, count, i < 0 ? -1 : slotOf(i));
  }

  /// Adds [login]; false when it is there already or the screen is full.
  bool add(String login) {
    if (!canAdd || _logins.contains(login)) return false;
    _logins.add(login);
    if (!multiviewLayoutsFor(count).contains(_layout)) {
      _layout = multiviewDefaultLayout(count);
    }
    notifyListeners();
    return true;
  }

  /// Removes [login]; the heard tile goes to the first one if it was it.
  void remove(String login) {
    final i = _logins.indexOf(login);
    if (i < 0) return;
    _logins.removeAt(i);
    if (i == _audio) {
      _audio = 0;
    } else if (i < _audio) {
      _audio--;
    }
    if (!multiviewLayoutsFor(count).contains(_layout)) {
      _layout = multiviewDefaultLayout(count);
    }
    notifyListeners();
  }

  /// Makes the tile [index] the heard (and in "one big" the big) one.
  void setAudio(int index) {
    if (index < 0 || index >= count || index == _audio) return;
    _audio = index;
    notifyListeners();
  }

  /// [layout] if it fits the number of tiles.
  void setLayout(MultiviewLayout layout) {
    if (layout == _layout || !multiviewLayoutsFor(count).contains(layout)) {
      return;
    }
    _layout = layout;
    notifyListeners();
  }

  /// The next layout for this number of tiles.
  void nextLayout() {
    final all = multiviewLayoutsFor(count);
    setLayout(all[(all.indexOf(_layout) + 1) % all.length]);
  }
}

/// What a tile shows.
enum MultiviewStreamStatus { loading, playing, offline, failed }

/// One live stream in a tile: a native player or an official embed.
abstract class MultiviewStream {
  ValueListenable<MultiviewStreamStatus> get status;

  /// Only the heard tile has sound.
  void setMuted(bool muted);

  /// The quality cap ([multiviewQuality]); a stream already at the variant
  /// it would pick stays as it is.
  void setQuality(String preference);

  /// The picture.
  Widget build(BuildContext context);

  void dispose();
}

/// Starts the stream of [login] muted or not, at the cap [quality].
typedef MultiviewStreamFactory = MultiviewStream Function(
  String login, {
  required bool muted,
  required String quality,
});

/// The picture height of a cap ([multiviewQuality]) for the embed: `720`
/// → 720; `source` or anything else → 0 (no cap).
int multiviewMaxHeight(String preference) => int.tryParse(preference) ?? 0;

/// Options of `new Twitch.Player(…)` for one multiview tile: the live
/// channel [login], muted unless it is the heard tile.
Map<String, Object> twitchMultiviewEmbedOptions(
  String login, {
  required bool muted,
  String parent = twitchEmbedParent,
}) => {
  'width': '100%', // l10n-ignore
  'height': '100%', // l10n-ignore
  'parent': [parent],
  'autoplay': true,
  'muted': muted,
  'channel': login,
};

/// The page of one embed tile. Like the single embed (TWI-005) it posts
/// `{"event": "ready|play|online|offline|error"}` to the `KinoluneEmbed`
/// channel; `window.kinolune.setMuted(bool)` and
/// `window.kinolune.setMaxHeight(int)` (0: no cap) follow the screen. The
/// options are JSON with `<` escaped, so no value closes the script.
String twitchMultiviewEmbedHtml(
  String login, {
  required bool muted,
  required int maxHeight,
  String parent = twitchEmbedParent,
}) {
  final options = jsonEncode(
    twitchMultiviewEmbedOptions(login, muted: muted, parent: parent),
  ).replaceAll('<', r'\u003c');
  return _multiviewPage
      .replaceFirst('/*OPTIONS*/null', options)
      .replaceFirst('/*MAX*/0', '${math.max(0, maxHeight)}');
}

// The Twitch player picks its quality by itself ("auto"); the cap sets the
// best variant not above it once the player knows its variants and again
// when the screen changes the cap.
const _multiviewPage = r"""<!DOCTYPE html>
<html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>html,body{margin:0;width:100%;height:100%;background:#000;overflow:hidden}
#player{width:100%;height:100%}</style></head>
<body><div id="player"></div>
<script src="https://player.twitch.tv/js/embed/v1.js"></script>
<script>
(function () {
  var options = /*OPTIONS*/null;
  var maxHeight = /*MAX*/0;
  function post(m) {
    try { KinoluneEmbed.postMessage(JSON.stringify(m)); } catch (e) {}
  }
  if (typeof Twitch === 'undefined' || !Twitch.Player) {
    post({event: 'error'});
    return;
  }
  var player = new Twitch.Player('player', options);
  function heightOf(q) {
    var m = /(\d+)p/.exec((q && (q.name || q.group)) || '');
    return m ? parseInt(m[1], 10) : 0;
  }
  function cap() {
    if (!maxHeight) return;
    try {
      var best = null, bestHeight = 0;
      (player.getQualities() || []).forEach(function (q) {
        var h = heightOf(q);
        if (h > 0 && h <= maxHeight && h > bestHeight) {
          best = q;
          bestHeight = h;
        }
      });
      if (best && player.getQuality() !== best.group) {
        player.setQuality(best.group);
      }
    } catch (e) {}
  }
  var E = Twitch.Player;
  player.addEventListener(E.READY, function () { post({event: 'ready'}); });
  player.addEventListener(E.PLAY, function () {
    cap();
    post({event: 'play'});
  });
  player.addEventListener(E.ONLINE, function () { post({event: 'online'}); });
  player.addEventListener(E.OFFLINE, function () { post({event: 'offline'}); });
  window.kinolune = {
    setMuted: function (m) { try { player.setMuted(!!m); } catch (e) {} },
    setMaxHeight: function (h) { maxHeight = h; cap(); }
  };
})();
</script></body></html>""";
