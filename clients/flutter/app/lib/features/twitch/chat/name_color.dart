// SPDX-License-Identifier: MPL-2.0
//
// Chat name colours (TWI-012): users pick any colour, many are unreadable
// on a dark background (#0000FF, #000000). Each colour is lightened (or
// darkened on a light background) until it reaches WCAG AA contrast
// (4.5:1), keeping its hue. Users without a colour get one of Twitch's
// default colours by login.

import 'package:flutter/painting.dart';

/// Minimum contrast of a name against the chat background (WCAG AA).
const double kNameMinContrast = 4.5;

/// Twitch's default name colours, picked by login for users without one.
const _defaults = <int>[
  0xFFFF0000, 0xFF0000FF, 0xFF008000, 0xFFB22222, 0xFFFF7F50, //
  0xFF9ACD32, 0xFFFF4500, 0xFF2E8B57, 0xFFDAA520, 0xFFD2691E,
  0xFF5F9EA0, 0xFF1E90FF, 0xFFFF69B4, 0xFF8A2BE2, 0xFF00FF7F,
];

/// WCAG contrast ratio of two opaque colours.
double contrastRatio(Color a, Color b) {
  final la = a.computeLuminance();
  final lb = b.computeLuminance();
  final hi = la > lb ? la : lb;
  final lo = la > lb ? lb : la;
  return (hi + 0.05) / (lo + 0.05);
}

/// Parses `#RRGGBB`; `null` when malformed.
Color? parseHexColor(String? hex) {
  if (hex == null) return null;
  final s = hex.startsWith('#') ? hex.substring(1) : hex;
  if (s.length != 6) return null;
  final v = int.tryParse(s, radix: 16);
  return v == null ? null : Color(0xFF000000 | v);
}

/// Name colours for one background, cached per input.
class NameColors {
  NameColors(this.background);

  final Color background;
  final _cache = <String, Color>{};

  /// Readable colour for a user's [hex] colour (or [login]'s default).
  Color of(String? hex, String login) {
    final key = hex ?? '@$login';
    return _cache[key] ??= readable(
      parseHexColor(hex) ?? _defaultFor(login),
      background,
    );
  }

  static Color _defaultFor(String login) {
    var h = 0;
    for (final c in login.codeUnits) {
      h = (h * 31 + c) & 0x7FFFFFFF;
    }
    return Color(_defaults[h % _defaults.length]);
  }
}

/// [color] with its lightness moved away from [background] until the
/// contrast is at least [minContrast]; the hue stays.
Color readable(
  Color color,
  Color background, {
  double minContrast = kNameMinContrast,
}) {
  if (contrastRatio(color, background) >= minContrast) return color;
  final darkBg = background.computeLuminance() < 0.5;
  var hsl = HSLColor.fromColor(color);
  for (var i = 0; i < 40; i++) {
    final l = (hsl.lightness + (darkBg ? 0.025 : -0.025)).clamp(0.0, 1.0);
    // Very saturated dark blues stay dim even at high lightness: also
    // drop some saturation on the way.
    hsl = hsl.withLightness(l).withSaturation(hsl.saturation * 0.985);
    final c = hsl.toColor();
    if (contrastRatio(c, background) >= minContrast) return c;
  }
  return darkBg ? const Color(0xFFFFFFFF) : const Color(0xFF000000);
}
