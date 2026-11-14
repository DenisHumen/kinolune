// SPDX-License-Identifier: MPL-2.0
//
// Autocomplete of the chat input (TWI-024): `@` + the start of a name
// offers recent chatters, `:` + at least two letters offers emotes of the
// channel and the global set; Tab completes the word at the cursor (an
// emote by its start, like on Twitch, or the highlighted suggestion).
// Pure functions: the input widget and tests share them.

import 'package:flutter/services.dart';

import 'chat_model.dart';

enum CompletionKind { mention, emote }

/// The word being typed and what it completes to.
class CompletionQuery {
  const CompletionQuery({
    required this.kind,
    required this.start,
    required this.end,
    required this.prefix,
  });

  final CompletionKind kind;

  /// The word's range in the text (including `@` or `:`).
  final int start;
  final int end;

  /// What the user typed after `@` / `:` (or the whole word for Tab).
  final String prefix;
}

/// One suggestion: a chatter's name or an emote.
class ChatSuggestion {
  const ChatSuggestion.mention(this.text) : emote = null;
  const ChatSuggestion.emote(ChatEmote this.emote) : text = '';

  final String text;
  final ChatEmote? emote;

  /// What replaces the word.
  String get insert => emote?.code ?? '@$text';
  String get label => emote?.code ?? text;
}

bool _isSpace(int c) => c == 0x20 || c == 0x0A || c == 0x09;

/// Most suggestions shown.
const kMaxSuggestions = 8;

/// The word before [cursor] if it asks for suggestions. With [tab] a plain
/// word of two or more characters completes to an emote too.
CompletionQuery? completionAt(String text, int cursor, {bool tab = false}) {
  if (cursor < 0 || cursor > text.length) return null;
  var start = cursor;
  while (start > 0 && !_isSpace(text.codeUnitAt(start - 1))) {
    start--;
  }
  if (start == cursor) return null;
  final word = text.substring(start, cursor);
  if (word.startsWith('@')) {
    return CompletionQuery(
      kind: CompletionKind.mention,
      start: start,
      end: cursor,
      prefix: word.substring(1),
    );
  }
  if (word.startsWith(':') && word.length >= 3) {
    return CompletionQuery(
      kind: CompletionKind.emote,
      start: start,
      end: cursor,
      prefix: word.substring(1),
    );
  }
  if (tab && word.length >= 2) {
    return CompletionQuery(
      kind: CompletionKind.emote,
      start: start,
      end: cursor,
      prefix: word,
    );
  }
  return null;
}

/// Suggestions for [query]: chatters by name start (most recent first),
/// emotes by code start (channel first), then by containing the text.
List<ChatSuggestion> suggestionsFor(
  CompletionQuery query, {
  required List<String> chatters,
  required List<ChatEmote> emotes,
  int limit = kMaxSuggestions,
}) {
  final p = query.prefix.toLowerCase();
  switch (query.kind) {
    case CompletionKind.mention:
      return [
        for (final c in chatters)
          if (c.toLowerCase().startsWith(p)) ChatSuggestion.mention(c),
      ].take(limit).toList();
    case CompletionKind.emote:
      final starts = <ChatSuggestion>[];
      final contains = <ChatSuggestion>[];
      for (final e in emotes) {
        final code = e.code.toLowerCase();
        if (code.startsWith(p)) {
          starts.add(ChatSuggestion.emote(e));
        } else if (p.length >= 3 && code.contains(p)) {
          contains.add(ChatSuggestion.emote(e));
        }
        if (starts.length >= limit) break;
      }
      return [...starts, ...contains].take(limit).toList();
  }
}

/// [value] with the word of [query] replaced by [s] and a space after it;
/// the cursor goes after the space.
TextEditingValue applySuggestion(
  TextEditingValue value,
  CompletionQuery query,
  ChatSuggestion s,
) {
  final text = value.text;
  final after = text.substring(query.end);
  final insert = after.startsWith(' ') ? s.insert : '${s.insert} ';
  final next = text.substring(0, query.start) + insert + after;
  final cursor = query.start + insert.length + (after.startsWith(' ') ? 1 : 0);
  return TextEditingValue(
    text: next,
    selection: TextSelection.collapsed(offset: cursor),
  );
}
