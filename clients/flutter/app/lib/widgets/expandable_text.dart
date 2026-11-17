// SPDX-License-Identifier: MPL-2.0
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';

/// Text cut to [maxLines] with a «More» / «Less» toggle when it does not fit
/// (overviews on the movie and show pages).
class ExpandableText extends StatefulWidget {
  const ExpandableText(
    this.text, {
    super.key,
    required this.maxLines,
    this.style,
  });

  final String text;
  final int maxLines;
  final TextStyle? style;

  @override
  State<ExpandableText> createState() => _ExpandableTextState();
}

class _ExpandableTextState extends State<ExpandableText> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return LayoutBuilder(
      builder: (context, box) {
        final painter = TextPainter(
          text: TextSpan(text: widget.text, style: widget.style),
          maxLines: widget.maxLines,
          textDirection: Directionality.of(context),
          textScaler: MediaQuery.textScalerOf(context),
        )..layout(maxWidth: box.maxWidth);
        final overflows = painter.didExceedMaxLines;
        painter.dispose();
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              widget.text,
              maxLines: _open ? null : widget.maxLines,
              overflow: _open ? TextOverflow.visible : TextOverflow.ellipsis,
              style: widget.style,
            ),
            if (overflows)
              TextButton(
                key: const ValueKey('expandable-text-toggle'),
                style: TextButton.styleFrom(
                  padding: EdgeInsets.zero,
                  minimumSize: const Size(0, 32),
                  tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                ),
                onPressed: () => setState(() => _open = !_open),
                child: Text(_open ? l.commonLess : l.commonMore),
              ),
          ],
        );
      },
    );
  }
}
