// SPDX-License-Identifier: MPL-2.0

/// A semver version of the app (`0.1.0-alpha.6`), ordered by semver
/// precedence: `0.1.0-alpha.6 < 0.1.0-alpha.10 < 0.1.0-beta.1 < 0.1.0`.
/// Build metadata (`+42`) is ignored, as in release tags.
class AppVersion implements Comparable<AppVersion> {
  const AppVersion._(this.core, this.pre, this.text);

  /// `[major, minor, patch]`.
  final List<int> core;

  /// Pre-release identifiers (`['alpha', '6']`); empty for a release.
  final List<String> pre;

  /// The parsed text without a leading `v` or `app-v`.
  final String text;

  bool get isPrerelease => pre.isNotEmpty;

  /// Parses `1.2.3`, `1.2.3-alpha.4`, `v1.2.3`, `app-v1.2.3+7`; null for
  /// anything else.
  static AppVersion? tryParse(String raw) {
    var s = raw.trim();
    if (s.startsWith('app-v')) s = s.substring(5);
    if (s.startsWith('v')) s = s.substring(1);
    final plus = s.indexOf('+');
    if (plus >= 0) s = s.substring(0, plus);
    final dash = s.indexOf('-');
    final coreText = dash >= 0 ? s.substring(0, dash) : s;
    final preText = dash >= 0 ? s.substring(dash + 1) : null;
    final parts = coreText.split('.');
    if (parts.length != 3) return null;
    final core = <int>[];
    for (final p in parts) {
      if (p.isEmpty || !_digits.hasMatch(p)) return null;
      core.add(int.parse(p));
    }
    final pre = <String>[];
    if (preText != null) {
      if (preText.isEmpty) return null;
      for (final id in preText.split('.')) {
        if (id.isEmpty || !_identifier.hasMatch(id)) return null;
        pre.add(id);
      }
    }
    return AppVersion._(core, pre, s);
  }

  static final _digits = RegExp(r'^[0-9]+$');
  static final _identifier = RegExp(r'^[0-9A-Za-z-]+$');

  @override
  int compareTo(AppVersion other) {
    for (var i = 0; i < 3; i++) {
      final c = core[i].compareTo(other.core[i]);
      if (c != 0) return c;
    }
    // A release ranks above its pre-releases.
    if (pre.isEmpty || other.pre.isEmpty) {
      return (pre.isEmpty ? 1 : 0) - (other.pre.isEmpty ? 1 : 0);
    }
    for (var i = 0; i < pre.length && i < other.pre.length; i++) {
      final a = pre[i], b = other.pre[i];
      final an = _digits.hasMatch(a), bn = _digits.hasMatch(b);
      final c = switch ((an, bn)) {
        (true, true) => int.parse(a).compareTo(int.parse(b)),
        // Numeric identifiers rank below alphanumeric ones.
        (true, false) => -1,
        (false, true) => 1,
        (false, false) => a.compareTo(b),
      };
      if (c != 0) return c;
    }
    return pre.length.compareTo(other.pre.length);
  }

  bool operator >(AppVersion other) => compareTo(other) > 0;
  bool operator <(AppVersion other) => compareTo(other) < 0;

  @override
  bool operator ==(Object other) =>
      other is AppVersion && compareTo(other) == 0;

  @override
  int get hashCode => Object.hash(Object.hashAll(core), Object.hashAll(pre));

  @override
  String toString() => text;
}
