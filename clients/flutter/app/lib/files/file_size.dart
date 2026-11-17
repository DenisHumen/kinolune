// SPDX-License-Identifier: MPL-2.0

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../screens/detail_sections.dart' show formatSize;

/// A file size for the file view: kilobytes below a megabyte (a photo, a
/// short clip), else as on the detail pages.
String fileSize(AppLocalizations l, Locale locale, int bytes) {
  if (bytes >= 1000 * 1000) return formatSize(l, locale, bytes);
  final kb = (bytes / 1000).ceil();
  return l.filesSizeKb(NumberFormat('0', locale.toLanguageTag()).format(kb));
}
