// SPDX-License-Identifier: MPL-2.0
//
// PIN flows shared by the picker, Settings and the profile editor (UI-015).

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart' show LibraryProfile;
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import 'pin_pad.dart';
import 'profile_avatar.dart';
import 'profile_controller.dart';

void _say(BuildContext context, String text) =>
    ScaffoldMessenger.maybeOf(context)
        ?.showSnackBar(SnackBar(content: Text(text)));

/// Says why a profile change was refused.
void sayProfileError(BuildContext context, Object error) {
  final l = context.l10n;
  _say(context, switch (error) {
    ProfileChangeException(error: ProfileChangeError.needsParentPin) =>
      l.profilesNeedsParentPin,
    ProfileChangeException(error: ProfileChangeError.deleteActive) =>
      l.profilesDeleteActive,
    _ => l.profilesChangeFailed('$error'),
  });
}

/// The parent PIN, unless it was entered in this visit to Settings or
/// there is none: true when the user may go on.
Future<bool> ensureParent(BuildContext context) async {
  final ctl = Services.read(context).profileControl;
  if (!ctl.manageLocked) return true;
  final l = context.l10n;
  return showPinDialog(
    context,
    title: l.profilesParentPinTitle,
    subtitle: l.profilesParentPinManage,
    limiter: ctl.limiter,
    check: ctl.unlockSettings,
  );
}

/// Makes sure a parent PIN exists: offers to set one on the active profile
/// (when it is not restricted). True when there is one now.
Future<bool> ensureParentPinExists(BuildContext context) async {
  final ctl = Services.read(context).profileControl;
  if (ctl.hasParentPin) return true;
  final l = context.l10n;
  final active = ctl.active;
  if (active == null || ProfileController.isRestricted(active)) {
    _say(context, l.profilesNeedsParentPin);
    return false;
  }
  final pin = await showNewPinDialog(
    context,
    title: l.profilesParentPinSetTitle,
    subtitle: l.profilesParentPinSetBody(profileName(l, active)),
  );
  if (pin == null) return false;
  try {
    await ctl.setPin(active.id, pin);
  } catch (e) {
    if (context.mounted) sayProfileError(context, e);
    return false;
  }
  return ctl.hasParentPin;
}

/// Switches to [target], asking for the parent PIN when leaving a
/// restricted profile and for the target's own PIN. True if switched.
Future<bool> switchProfileFlow(
  BuildContext context,
  LibraryProfile target,
) async {
  final ctl = Services.read(context).profileControl;
  final l = context.l10n;
  final needs = ctl.needsFor(target);
  if (needs.parentPin) {
    final ok = await showPinDialog(
      context,
      title: l.profilesParentPinTitle,
      subtitle: l.profilesParentPinLeave,
      limiter: ctl.limiter,
      check: ctl.verifyParent,
    );
    if (!ok || !context.mounted) return false;
  }
  if (needs.profilePin) {
    return showPinDialog(
      context,
      title: l.profilesPinFor(profileName(l, target)),
      limiter: ctl.limiter,
      check: (pin) => ctl.switchTo(target, pin: pin),
    );
  }
  final r = await ctl.switchTo(target);
  if (r != PinCheck.ok && context.mounted) {
    _say(context, l.profilesSwitchFailed);
  }
  return r == PinCheck.ok;
}
