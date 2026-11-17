// SPDX-License-Identifier: MPL-2.0
//
// A removal with «Отменить» (QA-015), the pattern of the files (UI-035):
// the thing goes at once, without a confirmation dialog, and the snack bar
// offers it back for a few seconds. Used where nothing on disk is lost:
// a smart playlist, a Twitch recording rule, a finished entry of a
// download list.

import 'package:material_ui/material_ui.dart';

/// How long the snack bar offers «Отменить».
const kUndoSnackDuration = Duration(seconds: 8);

/// Shows [message] with an «Отменить» action labelled [undoLabel] on
/// [messenger] (taken before the caller's page closes). Completes with
/// true when the user pressed it, false when the snack bar went away
/// otherwise (timeout, another message, no messenger).
Future<bool> showUndoSnack(
  ScaffoldMessengerState? messenger, {
  required String message,
  required String undoLabel,
}) async {
  if (messenger == null) return false;
  messenger.hideCurrentSnackBar();
  final reason = await messenger
      .showSnackBar(
        SnackBar(
          content: Text(message),
          duration: kUndoSnackDuration,
          // Goes away by itself even with an action: until then the
          // removal of a list entry waits.
          persist: false,
          action: SnackBarAction(
            key: const ValueKey('snack-undo'),
            label: undoLabel,
            onPressed: () {},
          ),
        ),
      )
      .closed;
  return reason == SnackBarClosedReason.action;
}
