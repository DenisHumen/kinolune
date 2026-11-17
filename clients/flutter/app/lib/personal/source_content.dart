// SPDX-License-Identifier: MPL-2.0
//
// UI-034, ADR-0099: no content type is picked anywhere. «Личное» is a
// place: a folder (or a whole source) goes there with «Перенести в Личное»
// in the source's file browser. What a source holds is shown in the
// sources list ([SourceContentMark]) and the browser knows which of its
// folders are personal ([PersonalPlaces]).

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';

/// Which places of one source are personal media, as far as the open
/// libraries tell: the whole source, and folders with their library.
/// Folders of a locked library are not listed (it shows nothing).
class PersonalPlaces {
  const PersonalPlaces({this.wholeSource = false, this.folders = const {}});

  /// The source is personal media (`content_type = personal`).
  final bool wholeSource;

  /// Folder (relative to the source root, no leading slash) → library.
  final Map<String, int> folders;

  /// The open library holding [folder] itself, if any.
  int? libraryOf(String folder) => folders[folder];

  /// Reads them for [sourceId].
  static Future<PersonalPlaces> read(
    PersonalMedia personal,
    String sourceId,
  ) async {
    final id = int.tryParse(sourceId);
    if (id == null) return const PersonalPlaces();
    final content = await personal.sourceContent(id);
    final folders = <String, int>{};
    for (final lib in await personal.libraries()) {
      for (final f in lib.folders) {
        if (f.sourceId != id) continue;
        final rel = f.path.replaceAll(RegExp(r'^/+|/+$'), '');
        if (rel.isNotEmpty) folders[rel] = lib.id;
      }
    }
    return PersonalPlaces(
      wholeSource: content == SourceContent.personal,
      folders: folders,
    );
  }
}

/// The content type of source [sourceId] in the sources list (UI-034):
/// «Личные медиа» or «Музыка»; nothing for movies and shows.
class SourceContentMark extends StatefulWidget {
  const SourceContentMark({super.key, required this.sourceId});

  final String sourceId;

  @override
  State<SourceContentMark> createState() => _SourceContentMarkState();
}

class _SourceContentMarkState extends State<SourceContentMark> {
  SourceContent? _content;
  StreamSubscription<void>? _sub;
  PersonalMedia? _personal;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final personal = Services.of(context).personal;
    if (identical(personal, _personal)) return;
    _sub?.cancel();
    _personal = personal;
    _sub = personal?.changes.listen((_) => _read());
    _read();
  }

  @override
  void didUpdateWidget(SourceContentMark old) {
    super.didUpdateWidget(old);
    if (old.sourceId != widget.sourceId) _read();
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _read() async {
    final personal = _personal;
    final id = int.tryParse(widget.sourceId);
    if (personal == null || id == null) return;
    try {
      final c = await personal.sourceContent(id);
      if (mounted && c != _content) setState(() => _content = c);
    } catch (_) {
      // Unknown source (deleted meanwhile): no mark.
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final (icon, text) = switch (_content) {
      SourceContent.personal => (Icons.lock_outline, l.sourceContentPersonal),
      SourceContent.music => (Icons.music_note_outlined, l.sourceContentMusic),
      _ => (null, null),
    };
    if (icon == null || text == null) return const SizedBox.shrink();
    final style = Theme.of(context).textTheme.labelSmall;
    return Padding(
      padding: const EdgeInsets.only(top: 2),
      child: Row(
        key: ValueKey('source-content-mark-${widget.sourceId}'),
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: 14, color: style?.color),
          const SizedBox(width: 4),
          Flexible(child: Text(text, style: style)),
        ],
      ),
    );
  }
}
