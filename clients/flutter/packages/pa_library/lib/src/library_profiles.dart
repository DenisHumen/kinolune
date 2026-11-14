// SPDX-License-Identifier: MPL-2.0

// Profiles and parental control (LIB-012). RustLibrary implements them on
// the Rust core; the v0 library has none (one implicit profile). The core
// enforces the parent PIN for profile management and limits wrong PINs
// (ADR-0055 amendment); RustLibrary keeps the parent token in memory.

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

/// The wrong-PIN backoff of the library: one for every PIN check, kept
/// across restarts (5 free tries, then 30 s doubling up to 1 h).
class PinAttempts {
  /// Wrong PINs since the last right one.
  final int failures;

  /// Until when no PIN is checked; null when PINs are checked now.
  final DateTime? lockedUntil;

  const PinAttempts({this.failures = 0, this.lockedUntil});

  @override
  bool operator ==(Object other) =>
      other is PinAttempts &&
      other.failures == failures &&
      other.lockedUntil == lockedUntil;

  @override
  int get hashCode => Object.hash(failures, lockedUntil);

  @override
  String toString() => 'PinAttempts($failures, $lockedUntil)';
}

/// The answer of the library to a PIN.
class PinResult {
  /// The PIN is right.
  final bool ok;

  /// Not checked: too many wrong PINs ([attempts]).
  final bool locked;

  /// The backoff after this check.
  final PinAttempts attempts;

  const PinResult({
    required this.ok,
    this.locked = false,
    this.attempts = const PinAttempts(),
  });
}

/// The library refused a profile change: a restricted profile is active
/// and the parent PIN was not proven (or its elevation expired), LIB-012.
class ProfileAccessDenied implements Exception {
  const ProfileAccessDenied([this.message = '']);
  final String message;

  @override
  String toString() => 'ProfileAccessDenied($message)';
}

/// Profiles of the library (LIB-012).
///
/// While a restricted (kid or filtered) profile is active and a parent PIN
/// exists, the library refuses the management calls — creating, changing,
/// reordering and deleting profiles, PINs, folder white lists, the
/// parent's own ratings — and leaving the profile with
/// [ProfileAccessDenied], until [verifyParentPin] (or [verifyProfilePin]
/// with a parent PIN) succeeds. That elevation lasts a few minutes without
/// use and ends with a profile switch or [endParentSession]. Every PIN
/// check shares one backoff ([pinAttempts]).
abstract interface class LibraryProfiles {
  /// Every profile in display order.
  Future<List<LibraryProfile>> profiles();

  /// The profile every query of the library runs for.
  Future<LibraryProfile> activeProfile();

  /// Makes [id] active and reloads the library views. A profile with a PIN
  /// needs [pin]; leaving a restricted profile needs the parent elevation
  /// unless [id] is a parent's profile. Null is returned when the switch is
  /// refused (a wrong or missing PIN, the backoff, no elevation). The
  /// elevation ends.
  Future<LibraryProfile?> switchProfile(int id, {String? pin});

  /// Checks a parent PIN (the PIN of any profile without restrictions); a
  /// right one elevates the library for the management calls.
  Future<PinResult> verifyParentPin(String pin);

  /// Checks the PIN of profile [id] (with [orParent], a parent PIN is right
  /// too); a profile without a PIN takes any. A parent PIN elevates the
  /// library.
  Future<PinResult> verifyProfilePin(
    int id,
    String pin, {
    bool orParent = false,
  });

  /// The wrong-PIN backoff now.
  Future<PinAttempts> pinAttempts();

  /// Ends the parent elevation at once.
  Future<void> endParentSession();

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
