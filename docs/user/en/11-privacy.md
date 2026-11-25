<!-- SPDX-License-Identifier: MPL-2.0 -->

# Privacy and external services

Kinolune has no accounts, no servers and no telemetry. Your library, progress, passwords and settings stay on your devices. The full text is the [privacy policy](../../legal/privacy-policy.md).

## Your own Kinolune media server

If you run the Kinolune media server (`alpha-media`), it is reachable on your home network only: devices with addresses from your router (192.168.x.x, 10.x.x.x and so on) connect, and the server drops connections from the internet, even if a port is forwarded on the router. The server never opens ports on the router itself (no UPnP). To allow this computer only, set `clients = "local"` in the config; to allow everybody, the internet included, `clients = "any"`. The latter is dangerous: only do it if you know why, for example behind your own VPN.

Until an administrator is created on the server, it can be managed only from the computer it runs on or with the one-time code from the `setup-code.txt` file in its data folder.

## Where passwords are kept

Network folder passwords, media server keys, and your Twitch and OpenSubtitles sign-ins are kept in the system's secure storage: Keychain (macOS), Android Keystore, Windows Credential Manager, Secret Service (Linux). If Linux has no such storage, Kinolune uses an encrypted file and tells you once. Settings → Privacy → Where passwords are stored shows which one is used.

## Which services the app contacts

Requests go straight from your device. A service sees your IP address and the request itself, never your files.

| Service | Why | How to turn it off |
|---|---|---|
| TMDB | Titles, posters, descriptions of movies and shows | Cannot be turned off: movies are not recognised without it. You can use your own key (Settings → Metadata) |
| AniList, Shikimori | Anime recognition | — |
| GitHub | Anime database updates (once a day), the signed app config and checks for new Kinolune versions | Settings → Metadata → Update the anime database; Settings → About → Updates → Check for updates automatically |
| MusicBrainz, Cover Art Archive | Music details and covers | — |
| Open Library | Audiobook descriptions and covers | — |
| YouTube, Vimeo | Trailers on a movie page when you have no trailer of your own; only once you open the page | Settings → Trailers → Trailers from the internet |
| OpenSubtitles | Subtitle search — only when you press "Find subtitles" | Simply do not use the search |
| SubDL | Subtitle search — only when you press "Find subtitles"; it gets the movie or show ID (IMDb/TMDB), season and episode, languages, and without an ID the title and year | Simply do not use the search |
| trace.moe | Recognising anime by its frames | **Off by default**; turned on in Settings → Metadata → Recognise anime by its frames |
| Twitch, 7TV, BTTV, FFZ | The Streams section and chat emotes | The section is off by default: Settings → Twitch streams → Show Twitch |

Your network sources (NAS, Jellyfin, Emby, Subsonic) and sync between devices stay between your own devices.

## What else you can do

- Settings → Privacy → Clear search history.
- Settings → Storage → Clear image cache.
- Settings → Diagnostics → Keep logs and crash reports — turn it off and the logs are deleted. Logs are never sent anywhere (see [Troubleshooting](12-troubleshooting.md)).
- Keep private videos in [Personal](05-personal-and-files.md) behind a PIN or password.
