# SPDX-License-Identifier: MPL-2.0
#
# Embeds the frameworks that packaging/app/macos/mpv-frameworks.sh put into
# macos/Frameworks (PKG-019): Mpv.framework (our LGPL-2.1 libmpv) and the
# LGPL-2.1 FFmpeg frameworks it links. media_kit_video links `-framework Mpv`
# from Frameworks/.symlinks/mpv/macos, a symlink to Frameworks that bundle.sh
# creates; both must exist before `pod install`.
frameworks_dir = File.join(__dir__, 'Frameworks', '.symlinks', 'mpv', 'macos')
unless File.exist?(File.join(frameworks_dir, 'Mpv.framework'))
  abort("media_kit_libs_macos_video: no Mpv.framework in #{frameworks_dir}. " \
        'Run packaging/app/macos/mpv-frameworks.sh clients/flutter/packages/media_kit_libs_macos_video/macos/Frameworks ' \
        '<libmpv-macos-arm64> [<libmpv-macos-x86_64>] first (see the package README).')
end

Pod::Spec.new do |s|
  s.name             = 'media_kit_libs_macos_video'
  s.version          = '1.1.4'
  s.summary          = 'LGPL libmpv and FFmpeg frameworks for package:media_kit (Kinolune)'
  s.homepage         = 'https://github.com/DenisHumen/project_alpha'
  s.license          = { :type => 'MPL-2.0' }
  s.author           = 'project_alpha contributors'
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'FlutterMacOS'
  s.vendored_frameworks = 'Frameworks/*.framework'
  s.platform = :osx, '12.0'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
  s.swift_version = '5.0'
end
