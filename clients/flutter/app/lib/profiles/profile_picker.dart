// SPDX-License-Identifier: MPL-2.0
//
// "Who is watching?" (UI-015, 01 §2.5 item 2): at start when there are two
// or more profiles (can be turned off in Settings), and from Settings →
// Switch profile. Profiles with a PIN show a lock; leaving a kid profile
// asks for the parent PIN.

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart' show LibraryProfile;
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import 'profile_avatar.dart';
import 'profile_flows.dart';

/// The picker as a whole screen: at start ([onDone] closes it) or pushed
/// from Settings (pops after a switch).
class ProfilePickerScreen extends StatelessWidget {
  const ProfilePickerScreen({super.key, this.atStart = false});

  /// Shown instead of the app at start; otherwise a pushed page.
  final bool atStart;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final picker = SafeArea(
      child: Center(
        child: SingleChildScrollView(
          padding: EdgeInsets.symmetric(
            horizontal: d.pageInsets.left,
            vertical: 24,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                l.profilesWhoIsWatching,
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.headlineMedium,
              ),
              SizedBox(height: d.isTv ? 24 : 32),
              ProfileGrid(
                onPicked: () {
                  if (!atStart && context.mounted) Navigator.of(context).pop();
                },
              ),
            ],
          ),
        ),
      ),
    );
    return Scaffold(
      key: ValueKey(atStart ? 'profile-picker-start' : 'profile-picker'),
      appBar: atStart ? null : AppBar(title: Text(l.profilesSwitch)),
      body: picker,
    );
  }
}

/// The profiles as large avatars.
class ProfileGrid extends StatelessWidget {
  const ProfileGrid({super.key, this.onPicked});

  final VoidCallback? onPicked;

  @override
  Widget build(BuildContext context) {
    final ctl = Services.of(context).profileControl;
    return ListenableBuilder(
      listenable: ctl,
      builder: (context, _) => Wrap(
        alignment: WrapAlignment.center,
        spacing: 24,
        runSpacing: 24,
        children: [
          for (final p in ctl.all)
            _ProfileTile(
              profile: p,
              active: p.id == ctl.active?.id,
              onTap: () async {
                if (await switchProfileFlow(context, p)) onPicked?.call();
              },
            ),
        ],
      ),
    );
  }
}

class _ProfileTile extends StatelessWidget {
  const _ProfileTile({
    required this.profile,
    required this.active,
    required this.onTap,
  });

  final LibraryProfile profile;
  final bool active;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final size = d.isTv ? 96.0 : 104.0;
    final name = profileName(l, profile);
    return KFocusable(
      key: ValueKey('profile-pick-${profile.id}'),
      onActivate: onTap,
      autofocus: active,
      radius: 16,
      light: false,
      focusScale: 1.06,
      semanticLabel: name,
      builder: (context, focused) => Container(
        width: size + 32,
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(16),
          color: focused ? KColors.focusFill : Colors.transparent,
        ),
        child: Column(
          children: [
            ProfileAvatar.of(profile, size: size),
            const SizedBox(height: 10),
            Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.titleMedium?.copyWith(
                color: focused ? KColors.focusLabel : null,
              ),
            ),
            const SizedBox(height: 4),
            SizedBox(
              height: 18,
              child: profile.hasPin
                  ? Icon(
                      Icons.lock_outline,
                      size: 16,
                      semanticLabel: l.profilesHasPin,
                      color: focused
                          ? KColors.focusLabel
                          : KColors.textSecondary,
                    )
                  : null,
            ),
          ],
        ),
      ),
    );
  }
}
