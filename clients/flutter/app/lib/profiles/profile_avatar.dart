// SPDX-License-Identifier: MPL-2.0
//
// Profile avatars (UI-015): a colour with the first letter of the name
// (`color:<c>`, as the first-run setup UI-004 stores it) or a colour with a
// picture (`icon:<i>:<c>`). The core keeps the reference as text.
//
// TWI-032: the Twitch avatar of the signed-in user,
// `twitch:<user_id>;<previous>;<profile_image_url>`. The previous colour or
// picture comes back when Twitch is signed out (and is the fallback while
// the picture loads). The public picture URL travels with the profile to
// the other devices (ADR-0102), each downloads the picture itself; no
// token ever does.

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart' show LibraryProfile;
import '../l10n/l10n.dart';
import '../services/local_profile.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/round_photo.dart';

/// Pictures for avatars, by index.
const kProfileAvatarIcons = <IconData>[
  Icons.face_rounded,
  Icons.pets_rounded,
  Icons.rocket_launch_rounded,
  Icons.star_rounded,
  Icons.sports_esports_rounded,
  Icons.music_note_rounded,
  Icons.cruelty_free_rounded,
  Icons.sports_soccer_rounded,
  Icons.palette_rounded,
  Icons.emoji_nature_rounded,
];

/// A parsed avatar reference.
class AvatarRef {
  const AvatarRef({
    this.color = 0,
    this.icon,
    this.twitchUserId,
    this.twitchUrl,
  });

  /// Index into [kProfileAvatarColors].
  final int color;

  /// Index into [kProfileAvatarIcons]; null = the initial of the name.
  final int? icon;

  /// The Twitch user whose avatar this is (TWI-032); null: [color] and
  /// [icon] only.
  final String? twitchUserId;

  /// The Twitch picture as last seen (`profile_image_url`).
  final String? twitchUrl;

  bool get isTwitch => twitchUserId != null;

  /// The longest reference the core keeps.
  static const maxLength = 256;

  static int _index(String? s, int length) {
    final i = int.tryParse(s ?? '') ?? 0;
    return i >= 0 && i < length ? i : 0;
  }

  /// `color:3`, `icon:2:5`, `twitch:<id>;<one of those>;<url>`; anything
  /// else is colour 0 with the initial.
  static AvatarRef parse(String? ref) {
    final r = ref ?? '';
    if (r.startsWith('twitch:')) {
      final parts = r.substring('twitch:'.length).split(';');
      final id = parts[0];
      if (id.isEmpty) return const AvatarRef();
      final base = parse(parts.length > 1 ? parts[1] : null);
      final url = parts.length > 2 ? parts.sublist(2).join(';') : '';
      return AvatarRef(
        color: base.color,
        icon: base.icon,
        twitchUserId: id,
        twitchUrl: url.isEmpty ? null : url,
      );
    }
    final parts = r.split(':');
    final colors = kProfileAvatarColors.length;
    if (parts.length == 2 && parts[0] == 'color') {
      return AvatarRef(color: _index(parts[1], colors));
    }
    if (parts.length == 3 && parts[0] == 'icon') {
      return AvatarRef(
        color: _index(parts[2], colors),
        icon: _index(parts[1], kProfileAvatarIcons.length),
      );
    }
    return const AvatarRef();
  }

  String get _base => icon == null ? 'color:$color' : 'icon:$icon:$color';

  String get ref {
    final id = twitchUserId;
    if (id == null) return _base;
    final bare = 'twitch:$id;$_base';
    final full = twitchUrl == null ? bare : '$bare;$twitchUrl';
    // A URL too long for the core: the picture comes from this device's
    // account only.
    return full.length <= maxLength ? full : bare;
  }

  /// The colour or picture under the Twitch avatar (the one to go back to).
  AvatarRef get base => AvatarRef(color: color, icon: icon);

  /// A colour or picture of one's own: no Twitch avatar any more.
  AvatarRef withColor(int c) => AvatarRef(color: c, icon: icon);
  AvatarRef withIcon(int? i) => AvatarRef(color: color, icon: i);

  /// The Twitch avatar of [userId] over this colour or picture.
  AvatarRef withTwitch(String userId, String? url) =>
      AvatarRef(color: color, icon: icon, twitchUserId: userId, twitchUrl: url);

  @override
  bool operator ==(Object other) =>
      other is AvatarRef &&
      other.color == color &&
      other.icon == icon &&
      other.twitchUserId == twitchUserId &&
      other.twitchUrl == twitchUrl;

  @override
  int get hashCode => Object.hash(color, icon, twitchUserId, twitchUrl);
}

/// Whether [ref] is a standard avatar (a colour or a picture): the Twitch
/// avatar is set on it by itself after signing in (TWI-032).
bool isStandardAvatar(String? ref) {
  final r = ref ?? '';
  return r.isEmpty || r.startsWith('color:') || r.startsWith('icon:');
}

/// The display name of [p]: the first profile has none until named.
String profileName(AppLocalizations l, LibraryProfile p) =>
    p.name.trim().isEmpty ? l.profilesDefaultName : p.name;

/// A round avatar.
class ProfileAvatar extends StatelessWidget {
  const ProfileAvatar({
    super.key,
    required this.avatar,
    required this.name,
    this.size = 40,
    this.kid = false,
  });

  ProfileAvatar.of(LibraryProfile p, {Key? key, double size = 40})
    : this(
        key: key,
        avatar: AvatarRef.parse(p.avatar),
        name: p.name,
        size: size,
        kid: p.isKid,
      );

  final AvatarRef avatar;
  final String name;
  final double size;

  /// Draws a small kid badge.
  final bool kid;

  @override
  Widget build(BuildContext context) {
    final runes = name.trim().runes;
    final initial = runes.isEmpty
        ? ''
        : String.fromCharCode(runes.first).toUpperCase();
    final icon = avatar.icon;
    final plain = CircleAvatar(
      radius: size / 2,
      backgroundColor:
          kProfileAvatarColors[avatar.color.clamp(
            0,
            kProfileAvatarColors.length - 1,
          )],
      child: icon != null
          ? Icon(
              kProfileAvatarIcons[icon],
              size: size * .55,
              color: Colors.white,
            )
          : initial.isEmpty
          ? Icon(Icons.person, size: size * .5, color: Colors.white)
          : Text(
              initial,
              style: TextStyle(
                fontSize: size * .42,
                fontWeight: FontWeight.w600,
                color: Colors.white,
              ),
            ),
    );
    final twitchId = avatar.twitchUserId;
    final Widget circle = twitchId == null
        ? plain
        : RoundPhoto(
            key: const ValueKey('profile-avatar-twitch'),
            url: avatar.twitchUrl,
            seed: 'avatar-twitch-$twitchId',
            size: size,
            fallback: plain,
          );
    if (!kid) return SizedBox.square(dimension: size, child: circle);
    return SizedBox.square(
      dimension: size,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned.fill(child: circle),
          Positioned(
            right: -2,
            bottom: -2,
            child: Container(
              padding: EdgeInsets.all(size * .05),
              decoration: BoxDecoration(
                color: KColors.bgBase,
                shape: BoxShape.circle,
              ),
              child: Icon(
                Icons.child_care_rounded,
                size: size * .28,
                color: KColors.textPrimary,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Colours and pictures to choose from (focusable for the remote).
class AvatarPicker extends StatelessWidget {
  const AvatarPicker({
    super.key,
    required this.value,
    required this.name,
    required this.onChanged,
  });

  final AvatarRef value;
  final String name;
  final ValueChanged<AvatarRef> onChanged;

  Widget _choice({
    required Key key,
    required bool selected,
    required String label,
    required VoidCallback onTap,
    required Widget child,
  }) => KFocusable(
    key: key,
    onActivate: onTap,
    radius: 999,
    light: false,
    semanticLabel: label,
    builder: (context, focused) => Container(
      padding: const EdgeInsets.all(3),
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        border: Border.all(
          color: selected || focused ? KColors.textPrimary : Colors.transparent,
          width: 2,
        ),
      ),
      child: child,
    ),
  );

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(l.profilesAvatarColor, style: theme.textTheme.titleSmall),
        const SizedBox(height: 8),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            for (var i = 0; i < kProfileAvatarColors.length; i++)
              _choice(
                key: ValueKey('profile-avatar-color-$i'),
                selected: value.color == i,
                label: l.onboardingProfileAvatarItem(i + 1),
                onTap: () => onChanged(value.withColor(i)),
                child: CircleAvatar(
                  radius: 16,
                  backgroundColor: kProfileAvatarColors[i],
                ),
              ),
          ],
        ),
        const SizedBox(height: 16),
        Text(l.profilesAvatarPicture, style: theme.textTheme.titleSmall),
        const SizedBox(height: 8),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            _choice(
              key: const ValueKey('profile-avatar-icon-none'),
              selected: value.icon == null,
              label: l.profilesAvatarInitial,
              onTap: () => onChanged(value.withIcon(null)),
              child: ProfileAvatar(
                avatar: value.withIcon(null),
                name: name,
                size: 32,
              ),
            ),
            for (var i = 0; i < kProfileAvatarIcons.length; i++)
              _choice(
                key: ValueKey('profile-avatar-icon-$i'),
                selected: value.icon == i,
                label: l.profilesAvatarPictureItem(i + 1),
                onTap: () => onChanged(value.withIcon(i)),
                child: ProfileAvatar(
                  avatar: value.withIcon(i),
                  name: name,
                  size: 32,
                ),
              ),
          ],
        ),
      ],
    );
  }
}
