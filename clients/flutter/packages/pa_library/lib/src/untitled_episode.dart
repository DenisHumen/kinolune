// SPDX-License-Identifier: MPL-2.0

/// The title of an episode that has none of its own: no title in the file
/// name and no provider match (BUG-014). The library has no UI strings, so
/// the app sets [UntitledEpisode.title] from its l10n («Серия 13»); English
/// until it does.
abstract final class UntitledEpisode {
  /// Title of untitled episode [number] (null: the number is unknown).
  static String Function(int? number) title = english;

  /// The default: «Episode 13».
  static String english(int? number) =>
      number == null ? 'Episode' : 'Episode $number';
}
