// SPDX-License-Identifier: MPL-2.0
//
// Local paths as the user sees them (BUG-022). The core stores a local
// folder as a `file://` URI (always `/`) and files under it as `/`-separated
// paths; on Windows they must show as `C:\Users\me\Movies\Dune.mkv`, not
// `C:/Users/me/Movies\Dune.mkv`. Network addresses (`smb://`, `https://`)
// are never touched. [context] defaults to this platform's `package:path`
// context; tests pass `p.windows` / `p.posix`.

import 'package:path/path.dart' as p;

/// Whether [s] is a network address (`smb://…`, `https://…`), not a path.
bool _isNetwork(String s) => s.contains('://') && !s.startsWith('file:');

/// [pathOrUri] (a path, possibly with mixed separators, or a `file://` URI)
/// in the native form of [context]: `\` on Windows, unchanged elsewhere.
/// Network addresses and unparseable URIs come back as they are.
String displayLocalPath(String pathOrUri, {p.Context? context}) {
  final ctx = context ?? p.context;
  if (_isNetwork(pathOrUri)) return pathOrUri;
  var s = pathOrUri;
  if (s.startsWith('file:')) {
    try {
      s = ctx.fromUri(Uri.parse(s));
    } on Object {
      return pathOrUri;
    }
  }
  return ctx.style == p.Style.windows ? s.replaceAll('/', r'\') : s;
}

/// [relPath] (`/`-separated, relative to a source root) under the local
/// folder [root], in the native form of [context]. A network [root] is
/// joined with `/` and is returned unchanged for an empty [relPath].
String joinLocalPath(String root, String relPath, {p.Context? context}) {
  final names = relPath.split('/').where((n) => n.isNotEmpty).toList();
  if (_isNetwork(root)) {
    if (names.isEmpty) return root;
    final base = root.endsWith('/') ? root.substring(0, root.length - 1) : root;
    return '$base/${names.join('/')}';
  }
  final ctx = context ?? p.context;
  final path = names.isEmpty ? root : ctx.joinAll([root, ...names]);
  return displayLocalPath(path, context: ctx);
}
