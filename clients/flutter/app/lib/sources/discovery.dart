// SPDX-License-Identifier: MPL-2.0

import 'dart:io';

import 'source_manager.dart';

/// A share found on the local network, offered at the top of the add flow.
class DiscoveredSource {
  const DiscoveredSource({
    required this.type,
    required this.name,
    required this.host,
    this.port,
    this.path,
    this.url,
    this.secure = true,
    this.stableName,
  });

  final SourceType type;

  /// Service instance name ("Living-room NAS").
  final String name;
  final String host;
  final int? port;

  /// Path from the TXT record (WebDAV `path=`), if any.
  final String? path;

  /// Address to connect to, built by the core from the IP address
  /// (`smb://192.168.1.2/`, `http://192.168.1.2:8080/dav`): `.local` names
  /// do not resolve everywhere (Android). Null before the address is known.
  final String? url;

  /// WebDAV over HTTPS (`_webdavs._tcp`) rather than HTTP (`_webdav._tcp`).
  final bool secure;

  /// The name the server keeps when its IP changes (BUG-004): the mDNS
  /// host (`nas.local`), the NetBIOS / WS-Discovery computer name, or the
  /// UDN of a DLNA server. Saved with the source, so the core finds the
  /// server again after DHCP gives it another address. Null: none known.
  final String? stableName;
}

/// Whether [text] is an IP address rather than a name.
bool isIpAddress(String text) =>
    InternetAddress.tryParse(text.replaceAll(RegExp(r'^\[|\]$'), '')) != null;

/// Our own server (`alpha-media`, mode B) announced on the LAN as
/// `_projectalpha._tcp` (ADR-0019). Offered by the first-run setup
/// (UI-004); connecting to it waits for server pairing (SRV-003).
class DiscoveredServer {
  const DiscoveredServer({
    required this.name,
    required this.host,
    this.port,
    this.id,
  });

  /// Service instance name ("Home server").
  final String name;

  /// Host name or, without one, the first address.
  final String host;
  final int? port;

  /// TXT `id`: the server's UUID.
  final String? id;
}

/// LAN discovery of shares (SMB, WebDAV, NFS) for the add flow.
///
/// `pa-discovery` (NET-009, mDNS/DNS-SD) through pa_core is
/// `CoreSourceDiscovery` (NET-020, core_discovery.dart). Without the Rust
/// core the app runs with [NoSourceDiscovery] and the add flow hides the
/// "On this network" section.
abstract class SourceDiscovery {
  /// False hides the "On this network" section.
  bool get supported;

  /// Current list of shares; emits again when one appears or goes away.
  Stream<List<DiscoveredSource>> watch();

  /// Our servers on the LAN (UI-004), like [watch].
  Stream<List<DiscoveredServer>> watchServers();
}

class NoSourceDiscovery implements SourceDiscovery {
  const NoSourceDiscovery();

  @override
  bool get supported => false;

  @override
  Stream<List<DiscoveredSource>> watch() => const Stream.empty();

  @override
  Stream<List<DiscoveredServer>> watchServers() => const Stream.empty();
}

/// A fixed list, for demo mode and tests.
class StaticSourceDiscovery implements SourceDiscovery {
  const StaticSourceDiscovery(this.sources, {this.servers = const []});

  final List<DiscoveredSource> sources;
  final List<DiscoveredServer> servers;

  @override
  bool get supported => true;

  @override
  Stream<List<DiscoveredSource>> watch() => Stream.value(sources);

  @override
  Stream<List<DiscoveredServer>> watchServers() => Stream.value(servers);
}
