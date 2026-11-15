// SPDX-License-Identifier: MPL-2.0

// The user's own categories, tags and 1–5 star ratings (LIB-022), per
// profile. RustLibrary implements them on the Rust core. Filter and sort
// by them with LibraryFilter.categories / tags / ratings and
// SortField.userRating.

import 'models.dart';

/// A category of the active profile.
class MediaCategory {
  final String id;
  final String name;

  /// Items in it the active profile may see.
  final int itemCount;

  const MediaCategory({
    required this.id,
    required this.name,
    this.itemCount = 0,
  });

  @override
  bool operator ==(Object other) =>
      other is MediaCategory &&
      other.id == id &&
      other.name == name &&
      other.itemCount == itemCount;

  @override
  int get hashCode => Object.hash(id, name, itemCount);

  @override
  String toString() => 'MediaCategory($id "$name" ×$itemCount)';
}

/// A tag with the number of visible items that have it.
class MediaTag {
  final String name;
  final int itemCount;

  const MediaTag(this.name, [this.itemCount = 0]);

  @override
  bool operator ==(Object other) =>
      other is MediaTag && other.name == name && other.itemCount == itemCount;

  @override
  int get hashCode => Object.hash(name, itemCount);

  @override
  String toString() => '$name: $itemCount';
}

/// Categories, tags and ratings of the active profile.
///
/// Item ids are [LibraryItem.id]s; every call also takes the `file:` key of
/// a file that is no library item (an unrecognized video). Items the
/// active profile may not see are never returned, and calls about one
/// throw as for a missing item.
abstract interface class LibraryLabels {
  /// Categories in the user's order.
  Future<List<MediaCategory>> categories();

  /// Throws if the name is in use (any case).
  Future<MediaCategory> createCategory(String name);
  Future<void> renameCategory(String id, String name);

  /// Deletes a category; its items stay in the library.
  Future<void> deleteCategory(String id);

  /// Items of a category, most recently added first.
  Future<List<LibraryItem>> categoryItems(String id);

  /// Categories that hold an item.
  Future<List<MediaCategory>> itemCategories(String itemId);

  /// Puts an item into a category or takes it out; whether it changed.
  Future<bool> setInCategory(String id, String itemId, bool on);

  /// Tags of visible items, by name.
  Future<List<MediaTag>> tags();

  /// Autocomplete: tags that start with [prefix], then those that contain
  /// it (case-insensitive).
  Future<List<MediaTag>> suggestTags(String prefix, {int limit = 10});

  /// Tags of an item, by name.
  Future<List<String>> itemTags(String itemId);

  /// Makes [tags] the tags of an item; whether anything changed.
  Future<bool> setItemTags(String itemId, List<String> tags);

  /// Items with a tag, most recently tagged first.
  Future<List<LibraryItem>> tagItems(String tag);

  /// Stars 1–5 the active profile gave an item, null if none.
  Future<int?> itemRating(String itemId);

  /// Rates an item 1–5 stars or clears its rating (null).
  Future<void> setItemRating(String itemId, int? stars);
}
