// SPDX-License-Identifier: MPL-2.0

// Profiles and parental control (LIB-012). RustLibrary implements them on
// the Rust core; the v0 library has none (one implicit profile).

/// A household profile. Progress, favorites, lists and view settings are
/// kept per profile; the library answers every query for the active one.
class LibraryProfile {
  final int id;

  /// Stable id shared by the devices of a household (`default` for the
  /// first profile); progress sync and handoff use it.
  final String syncId;

  /// Empty for the first profile until it is named: the app shows a
  /// localized default name.
  final String name;

  /// Avatar reference chosen by the app, e.g. `color:3`.
  final String? avatar;
  final bool isKid;

  /// Switching to the profile needs its PIN.
  final bool hasPin;

  /// Rating limit as a minimum viewer age; null = no limit.
  final int? maxAge;

  /// ISO 3166-1 alpha-2 country whose rating wins; '' = the strictest.
  final String ratingCountry;

  /// Video without a known rating is hidden.
  final bool hideUnrated;

  /// Posters, backdrops and stills are not shown.
  final bool hideArtwork;

  /// Only the folders of [LibraryProfiles.profileFolders] are visible.
  final bool restrictFolders;

  /// Any filter hides something from the profile.
  final bool isRestricted;

  const LibraryProfile({
    required this.id,
    required this.syncId,
    required this.name,
    this.avatar,
    this.isKid = false,
    this.hasPin = false,
    this.maxAge,
    this.ratingCountry = '',
    this.hideUnrated = false,
    this.hideArtwork = false,
    this.restrictFolders = false,
    this.isRestricted = false,
  });

  /// The editable part of the profile.
  ProfileSettings get settings => ProfileSettings(
    name: name,
    avatar: avatar,
    isKid: isKid,
    maxAge: maxAge,
    ratingCountry: ratingCountry,
    hideUnrated: hideUnrated,
    hideArtwork: hideArtwork,
    restrictFolders: restrictFolders,
  );

  @override
  bool operator ==(Object other) =>
      other is LibraryProfile &&
      other.id == id &&
      other.syncId == syncId &&
      other.hasPin == hasPin &&
      other.settings == settings;

  @override
  int get hashCode => Object.hash(id, syncId, hasPin, settings);

  @override
  String toString() => 'LibraryProfile($id, $name)';
}

/// Editable settings of a profile.
class ProfileSettings {
  final String name;
  final String? avatar;
  final bool isKid;
  final int? maxAge;
  final String ratingCountry;
  final bool hideUnrated;
  final bool hideArtwork;
  final bool restrictFolders;

  const ProfileSettings({
    required this.name,
    this.avatar,
    this.isKid = false,
    this.maxAge,
    this.ratingCountry = '',
    this.hideUnrated = false,
    this.hideArtwork = false,
    this.restrictFolders = false,
  });

  /// A kid profile with the safe defaults: rated up to [maxAge], unrated
  /// video and artwork hidden, only white-listed folders.
  const ProfileSettings.kid({
    required this.name,
    this.avatar,
    this.maxAge = 12,
    this.ratingCountry = '',
  }) : isKid = true,
       hideUnrated = true,
       hideArtwork = false,
       restrictFolders = true;

  ProfileSettings copyWith({
    String? name,
    String? avatar,
    bool? isKid,
    int? Function()? maxAge,
    String? ratingCountry,
    bool? hideUnrated,
    bool? hideArtwork,
    bool? restrictFolders,
  }) => ProfileSettings(
    name: name ?? this.name,
    avatar: avatar ?? this.avatar,
    isKid: isKid ?? this.isKid,
    maxAge: maxAge == null ? this.maxAge : maxAge(),
    ratingCountry: ratingCountry ?? this.ratingCountry,
    hideUnrated: hideUnrated ?? this.hideUnrated,
    hideArtwork: hideArtwork ?? this.hideArtwork,
    restrictFolders: restrictFolders ?? this.restrictFolders,
  );

  @override
  bool operator ==(Object other) =>
      other is ProfileSettings &&
      other.name == name &&
      other.avatar == avatar &&
      other.isKid == isKid &&
      other.maxAge == maxAge &&
      other.ratingCountry == ratingCountry &&
      other.hideUnrated == hideUnrated &&
      other.hideArtwork == hideArtwork &&
      other.restrictFolders == restrictFolders;

  @override
  int get hashCode => Object.hash(
    name,
    avatar,
    isKid,
    maxAge,
    ratingCountry,
    hideUnrated,
    hideArtwork,
    restrictFolders,
  );
}

/// One folder of a profile's white list.
class ProfileFolder {
  /// `MediaSource.id` of the source.
  final String sourceId;

  /// Folder relative to the source root (`Movies/Kids`); '' = all of it.
  final String relPath;

  const ProfileFolder({required this.sourceId, this.relPath = ''});

  @override
  bool operator ==(Object other) =>
      other is ProfileFolder &&
      other.sourceId == sourceId &&
      other.relPath == relPath;

  @override
  int get hashCode => Object.hash(sourceId, relPath);

  @override
  String toString() => 'ProfileFolder($sourceId, $relPath)';
}

/// One content rating of a movie or show.
class ContentRating {
  /// `tmdb`, or `user` for the parent's own rating.
  final String provider;

  /// ISO 3166-1 alpha-2; '' for the parent's rating.
  final String country;

  /// As the provider writes it: `PG-13`, `16+`.
  final String certification;

  /// Minimum viewer age; null if the rating is not understood.
  final int? minAge;

  const ContentRating({
    required this.provider,
    required this.country,
    required this.certification,
    this.minAge,
  });

  /// The parent's own rating.
  bool get isUser => provider == 'user';
}

/// Profiles of the library (LIB-012).
abstract interface class LibraryProfiles {
  /// Every profile in display order.
  Future<List<LibraryProfile>> profiles();

  /// The profile every query of the library runs for.
  Future<LibraryProfile> activeProfile();

  /// Makes [id] active and reloads the library views. A profile with a PIN
  /// needs [pin]: null is returned for a wrong or missing one.
  Future<LibraryProfile?> switchProfile(int id, {String? pin});

  /// True if [pin] is the PIN of profile [id], or it has none.
  Future<bool> verifyProfilePin(int id, String? pin);

  Future<LibraryProfile> createProfile(ProfileSettings settings);

  /// Replaces the settings of a profile; the active one applies them at
  /// once.
  Future<LibraryProfile> updateProfile(int id, ProfileSettings settings);

  /// Deletes a profile with its progress and lists. The last one stays.
  Future<void> deleteProfile(int id);

  /// Sets (4–8 digits) or removes (null) the PIN of a profile.
  Future<void> setProfilePin(int id, String? pin);

  Future<List<ProfileFolder>> profileFolders(int id);

  /// Replaces the white list (used while [ProfileSettings.restrictFolders]).
  Future<void> setProfileFolders(int id, List<ProfileFolder> folders);

  /// Content ratings of a movie or show, the parent's own first.
  Future<List<ContentRating>> contentRatings(String itemId);

  /// Sets the parent's own rating of a movie or show as a minimum age, or
  /// removes it.
  Future<void> setUserRating(String itemId, int? minAge);
}
