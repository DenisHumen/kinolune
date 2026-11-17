// SPDX-License-Identifier: MPL-2.0
//
// UI-034: «Что в источнике» everywhere a source is added or edited — the
// source form, the SMB wizard (UI-028), the Jellyfin and Emby forms. The
// choice is written to the core before the first scan of a new source, so
// personal files are never recognized; changing it later drops the old
// automatic matches (to personal) or scans the source again (back to
// movies and shows, or to music). A personal source whose library is
// locked keeps its type until the library is opened.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import 'personal_screen.dart' show SourceContentPicker;

/// What a source holds, as a form edits it.
class SourceContentController extends ChangeNotifier {
  SourceContent _content = SourceContent.video;
  int? _library;
  SourceContent _initial = SourceContent.video;
  int? _initialLibrary;
  bool _locked = false;
  bool _loaded = false;

  /// The choice.
  SourceContent get content => _content;

  /// The personal library a personal source joins.
  int? get libraryId => _library;

  /// The source is personal media of a locked library: its type cannot be
  /// changed now.
  bool get locked => _locked;

  /// The type of an existing source is known (a new one: always).
  bool get loaded => _loaded;

  bool get isPersonal => _content == SourceContent.personal;

  /// A personal source needs a library (unless it is locked and stays
  /// as it is).
  bool get valid => _locked || !isPersonal || _library != null;

  /// Something to write to the core.
  bool get changed =>
      _content != _initial ||
      (isPersonal && _library != null && _library != _initialLibrary);

  void set(SourceContent content, int? libraryId) {
    _content = content;
    _library = content == SourceContent.personal ? libraryId : null;
    notifyListeners();
  }

  /// A new source: movies and shows until the user picks something else.
  void startNew() {
    _loaded = true;
    notifyListeners();
  }

  /// Reads the type of existing source [sourceId] and, for personal media,
  /// the open library that holds it.
  Future<void> load(PersonalMedia personal, String sourceId) async {
    final id = int.tryParse(sourceId);
    if (id == null) {
      startNew();
      return;
    }
    final content = await personal.sourceContent(id);
    int? library;
    var locked = false;
    if (content == SourceContent.personal) {
      final holder = (await personal.libraries())
          .where((l) => l.folders.any((f) => f.sourceId == id && f.path == '/'))
          .firstOrNull;
      library = holder?.id;
      // Folders are listed only while a library is open: none found means
      // the source's library is locked.
      locked = holder == null;
    }
    _initial = _content = content;
    _initialLibrary = _library = library;
    _locked = locked;
    _loaded = true;
    notifyListeners();
  }

  /// Writes the choice for [sourceId] if it changed. A new source: call it
  /// before its first scan. An edited source that stops being personal
  /// media (or turns to music) is scanned again.
  Future<void> apply(
    AppServices services,
    String sourceId, {
    required bool editing,
  }) async {
    final personal = services.personal;
    final id = int.tryParse(sourceId);
    if (personal == null || id == null || !changed || _locked) return;
    await personal.setSourceContent(id, _content, libraryId: _library);
    _initial = _content;
    _initialLibrary = _library;
    if (editing && !isPersonal) unawaited(services.scan(sourceId: sourceId));
  }
}

/// The «Что в источнике» field of [controller]; nothing without personal
/// media (v0 library) or before an existing source's type is read.
class SourceContentField extends StatelessWidget {
  const SourceContentField({
    super.key,
    required this.controller,
    this.showErrors = false,
  });

  final SourceContentController controller;

  /// The form was submitted: say what is missing.
  final bool showErrors;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      if (Services.of(context).personal == null || !controller.loaded) {
        return const SizedBox.shrink();
      }
      if (controller.locked) {
        return Column(
          key: const ValueKey('source-content-locked'),
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              context.l10n.sourceContentLabel,
              style: Theme.of(context).textTheme.labelLarge,
            ),
            const SizedBox(height: 4),
            Text(context.l10n.sourceContentPersonal),
            Text(
              context.l10n.sourceContentLocked,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ],
        );
      }
      return SourceContentPicker(
        content: controller.content,
        libraryId: controller.libraryId,
        showErrors: showErrors,
        onChanged: controller.set,
      );
    },
  );
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
