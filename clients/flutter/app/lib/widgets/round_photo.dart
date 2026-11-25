// SPDX-License-Identifier: MPL-2.0
//
// A round photo over a fallback (TWI-032): the Twitch avatar of the
// account and of a profile. The picture comes from the core's disk cache
// (LIB-013) at the size on screen; while it loads, when there is none or
// when it fails, the fallback (a colour with the initial) stays visible.

import 'package:material_ui/material_ui.dart';

import 'artwork.dart';
import 'artwork_image.dart';

class RoundPhoto extends StatelessWidget {
  const RoundPhoto({
    super.key,
    required this.url,
    required this.seed,
    required this.size,
    required this.fallback,
  });

  /// http(s), a local file or the demo scheme; null: only [fallback].
  final String? url;

  /// Seeds the demo picture.
  final String seed;
  final double size;
  final Widget fallback;

  @override
  Widget build(BuildContext context) {
    final u = url;
    return SizedBox.square(
      dimension: size,
      child: Stack(
        fit: StackFit.expand,
        children: [
          fallback,
          if (u != null && u.isNotEmpty)
            ClipOval(
              child: u.startsWith(kDemoArtScheme)
                  ? GeneratedArt(seed: seed, style: ArtStyle.album)
                  : Image(
                      image: artworkProvider(
                        u,
                        width: decodeWidth(context, size),
                      ),
                      fit: BoxFit.cover,
                      gaplessPlayback: true,
                      errorBuilder: (_, _, _) => const SizedBox.shrink(),
                    ),
            ),
        ],
      ),
    );
  }
}
