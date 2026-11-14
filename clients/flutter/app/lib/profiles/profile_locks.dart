// SPDX-License-Identifier: MPL-2.0
//
// Locks of UI-015 (01 §1.5, §3 item 6):
// * [SectionLock] — Settings of a restricted profile and Search with
//   Search Lock show a PIN pad instead of their pages (the pages are not
//   even built) until the parent PIN is entered;
// * [AppLockScreen] — App Lock: covers the app when it starts or comes
//   back from the background.

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import 'pin_pad.dart';
import 'profile_avatar.dart';
import 'profile_controller.dart';

enum LockKind { settings, search }

/// Shows [child] only when the section is not locked.
class SectionLock extends StatelessWidget {
  const SectionLock({super.key, required this.kind, required this.child});

  final LockKind kind;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final ctl = Services.of(context).profileControl;
    return ListenableBuilder(
      listenable: ctl,
      builder: (context, _) {
        final locked = switch (kind) {
          LockKind.settings => ctl.settingsLocked,
          LockKind.search => ctl.searchLocked,
        };
        if (!locked) return child;
        return _LockedPage(kind: kind, controller: ctl);
      },
    );
  }
}

class _LockedPage extends StatelessWidget {
  const _LockedPage({required this.kind, required this.controller});

  final LockKind kind;
  final ProfileController controller;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    return Material(
      key: ValueKey('profiles-gate-${kind.name}'),
      color: KColors.bgBase,
      child: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: EdgeInsets.symmetric(
              horizontal: d.pageInsets.left,
              vertical: 16,
            ),
            child: PinPad(
              // On TV the menu keeps the focus; Down reaches the pad.
              autofocus: !d.isTv,
              title: l.profilesParentPinTitle,
              subtitle: switch (kind) {
                LockKind.settings => l.profilesGateSettings,
                LockKind.search => l.profilesGateSearch,
              },
              limiter: controller.limiter,
              onSubmit: switch (kind) {
                LockKind.settings => controller.unlockSettings,
                LockKind.search => controller.unlockSearch,
              },
            ),
          ),
        ),
      ),
    );
  }
}

/// App Lock: the active profile's avatar and the PIN pad over everything.
class AppLockScreen extends StatelessWidget {
  const AppLockScreen({super.key, required this.controller});

  final ProfileController controller;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final active = controller.active;
    return Material(
      key: const ValueKey('profiles-app-lock'),
      color: KColors.bgBase,
      child: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: EdgeInsets.symmetric(
              horizontal: d.pageInsets.left,
              vertical: 16,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (active != null && !d.isTv) ...[
                  ProfileAvatar.of(active, size: 64),
                  const SizedBox(height: 16),
                ],
                PinPad(
                  title: l.profilesAppLockTitle,
                  subtitle: active == null
                      ? null
                      : l.profilesAppLockBody(profileName(l, active)),
                  limiter: controller.limiter,
                  onSubmit: controller.unlockApp,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
