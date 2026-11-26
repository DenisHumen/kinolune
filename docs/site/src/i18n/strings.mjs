// SPDX-License-Identifier: MPL-2.0
// Website texts (SITE-001). Every locale has the same keys; scripts/check.mjs
// fails the build otherwise. Ukrainian is the product's first language,
// Russian and English follow (owner, 2026-11-25).

export const locales = ['uk', 'en', 'ru'];

/** Name of each language in itself, for the language switcher. */
export const languageNames = { uk: 'Українська', en: 'English', ru: 'Русский' };

const repo = 'https://github.com/DenisHumen/kinolune';

export const links = {
  repo,
  releases: `${repo}/releases`,
  discussions: `${repo}/discussions`,
  issues: `${repo}/issues`,
  contributing: `${repo}/blob/main/.github/CONTRIBUTING.md`,
  license: `${repo}/blob/main/LICENSE`,
  roadmap: `${repo}/blob/main/docs/ROADMAP.md`,
};

export const commands = {
  apt: [
    'curl -fsSL https://denishumen.github.io/kinolune/apt/kinolune.sources | sudo tee /etc/apt/sources.list.d/kinolune.sources >/dev/null',
    'sudo apt update && sudo apt install kinolune',
  ],
  dnf: [
    'curl -fsSL https://denishumen.github.io/kinolune/rpm/kinolune.repo | sudo tee /etc/yum.repos.d/kinolune.repo >/dev/null',
    'sudo dnf install kinolune',
  ],
};

export const strings = {
  uk: {
    'meta.title': 'Kinolune — вільний медіаплеєр для дому',
    'meta.description':
      'Фільми, серіали, аніме, музика і Twitch на всіх ваших пристроях — без хмари й акаунтів. Відкритий код, MPL-2.0.',
    'nav.label': 'Розділи сайту',
    'nav.features': 'Можливості',
    'nav.install': 'Встановити',
    'nav.opensource': 'Відкритий код',
    'nav.donate': 'Підтримати',
    'nav.github': 'GitHub',
    'lang.label': 'Мова',
    'skip': 'Перейти до змісту',
    'hero.badge': 'Альфа-версія',
    'hero.title': 'Ваша медіатека. Ваші пристрої. Без хмари.',
    'hero.lead':
      'Kinolune сам розпізнає фільми, серіали й аніме у ваших папках і мережевих дисках та відтворює майже будь-який формат — на компʼютері, телефоні й телевізорі.',
    'hero.download': 'Завантажити',
    'hero.source': 'Код на GitHub',
    'hero.note': 'Безкоштовно і з відкритим кодом. Без реклами, акаунтів і стеження.',
    'features.title': 'Можливості',
    'features.recognition.title': 'Розпізнає сам',
    'features.recognition.text':
      'Точно визначає фільми, серії та аніме за іменами файлів і папок. Постери, описи й порядок серій — без ручної роботи.',
    'features.playback.title': 'Грає майже все',
    'features.playback.text':
      'Майже будь-які відео- та аудіоформати, HDR, субтитри, передача звуку на ресивер без перекодування.',
    'features.sources.title': 'Звідки завгодно',
    'features.sources.text':
      'Локальні папки, SMB, NFS, WebDAV, хмарні диски, Plex, Jellyfin, Emby і власний медіасервер.',
    'features.music.title': 'Музика',
    'features.music.text':
      'Окремий плеєр: mp3, flac, alac, opus, dsd, відтворення без пауз між треками, тексти пісень.',
    'features.twitch.title': 'Twitch',
    'features.twitch.text': 'Ефіри з чатом, записи, улюблені стрімери — в одному застосунку.',
    'features.devices.title': 'Усі пристрої, без хмари',
    'features.devices.text':
      'Linux, Windows, macOS, Android і Android TV. Прогрес і медіатека синхронізуються у вашій домашній мережі, а не через наші сервери.',
    'shots.title': 'Як це виглядає',
    'shots.desktop': 'Головний екран на компʼютері',
    'shots.phone': 'Kinolune на телефоні',
    'shots.tv': 'Kinolune на телевізорі',
    'shots.note': 'На знімках — демо-медіатека з власними обкладинками-заглушками.',
    'install.title': 'Встановити',
    'install.lead':
      'Збірки для всіх платформ — у GitHub Releases: Linux (.deb, .rpm, AppImage), Windows (інсталятор), macOS (.dmg) і Android (APK).',
    'install.releases': 'Усі завантаження',
    'install.linux': 'Linux: з репозиторію, з оновленнями разом із системою',
    'install.apt': 'Ubuntu 24.04+, Debian 13+, Linux Mint 22+ (apt)',
    'install.dnf': 'Дистрибутиви з dnf (RPM)',
    'install.updates':
      'Далі Kinolune оновлюється разом із системою (sudo apt upgrade, sudo dnf upgrade). Пакети .deb і .rpm з релізу додають цей самий репозиторій.',
    'install.copy': 'Копіювати',
    'install.copied': 'Скопійовано',
    'oss.title': 'Відкритий код і приватність',
    'oss.text':
      'Увесь код Kinolune відкритий під ліцензією MPL-2.0. Застосунок працює локально: ваша медіатека, історія переглядів і паролі лишаються на ваших пристроях. У нас немає серверів, які щось про вас знають.',
    'oss.site': 'Цей сайт теж без cookies, лічильників і сторонніх скриптів.',
    'oss.license': 'Ліцензія MPL-2.0',
    'oss.roadmap': 'План розробки',
    'oss.contribute': 'Як долучитися',
    'donate.title': 'Підтримати проєкт',
    'donate.text':
      'Kinolune живе на добровільні пожертви. Майданчики для пожертв ще підключаються — посилання зʼявляться тут. Уже зараз дуже допомагають звіти про помилки, приклади неправильного розпізнавання і pull request’и.',
    'donate.issue': 'Повідомити про помилку',
    'donate.discuss': 'Обговорення',
    'footer.license': 'Kinolune — вільне програмне забезпечення під ліцензією MPL-2.0.',
    'footer.trademarks': 'Назви сторонніх продуктів і сервісів належать їхнім власникам.',
    'notfound.title': 'Сторінку не знайдено',
    'notfound.text': 'Такої сторінки немає. Можливо, посилання застаріло.',
    'notfound.home': 'На головну',
  },
  en: {
    'meta.title': 'Kinolune — a free media player for your home',
    'meta.description':
      'Movies, shows, anime, music and Twitch on all your devices — no cloud, no accounts. Open source, MPL-2.0.',
    'nav.label': 'Site sections',
    'nav.features': 'Features',
    'nav.install': 'Install',
    'nav.opensource': 'Open source',
    'nav.donate': 'Support',
    'nav.github': 'GitHub',
    'lang.label': 'Language',
    'skip': 'Skip to content',
    'hero.badge': 'Alpha',
    'hero.title': 'Your library. Your devices. No cloud.',
    'hero.lead':
      'Kinolune recognises the movies, shows and anime in your folders and network drives by itself and plays almost any format — on your computer, phone and TV.',
    'hero.download': 'Download',
    'hero.source': 'Code on GitHub',
    'hero.note': 'Free and open source. No ads, no accounts, no tracking.',
    'features.title': 'Features',
    'features.recognition.title': 'Recognises by itself',
    'features.recognition.text':
      'Accurately identifies movies, episodes and anime from file and folder names. Posters, plots and episode order with no manual work.',
    'features.playback.title': 'Plays almost anything',
    'features.playback.text':
      'Almost any video and audio format, HDR, subtitles, audio passthrough to your receiver without re-encoding.',
    'features.sources.title': 'From anywhere',
    'features.sources.text':
      'Local folders, SMB, NFS, WebDAV, cloud drives, Plex, Jellyfin, Emby and its own media server.',
    'features.music.title': 'Music',
    'features.music.text':
      'A dedicated player: mp3, flac, alac, opus, dsd, gapless playback, lyrics.',
    'features.twitch.title': 'Twitch',
    'features.twitch.text': 'Live streams with chat, VODs and favourite streamers — in the same app.',
    'features.devices.title': 'Every device, no cloud',
    'features.devices.text':
      'Linux, Windows, macOS, Android and Android TV. Progress and library sync over your home network, not through our servers.',
    'shots.title': 'What it looks like',
    'shots.desktop': 'Home screen on a computer',
    'shots.phone': 'Kinolune on a phone',
    'shots.tv': 'Kinolune on a TV',
    'shots.note': 'The screenshots show a demo library with our own placeholder covers.',
    'install.title': 'Install',
    'install.lead':
      'Builds for every platform are in GitHub Releases: Linux (.deb, .rpm, AppImage), Windows (installer), macOS (.dmg) and Android (APK).',
    'install.releases': 'All downloads',
    'install.linux': 'Linux: from the repository, updated with the system',
    'install.apt': 'Ubuntu 24.04+, Debian 13+, Linux Mint 22+ (apt)',
    'install.dnf': 'Distributions with dnf (RPM)',
    'install.updates':
      'After that Kinolune updates with the system (sudo apt upgrade, sudo dnf upgrade). The .deb and .rpm files from a release add the same repository.',
    'install.copy': 'Copy',
    'install.copied': 'Copied',
    'oss.title': 'Open source and privacy',
    'oss.text':
      'All of Kinolune’s code is open under the MPL-2.0 licence. The app runs locally: your library, watch history and passwords stay on your devices. We have no servers that know anything about you.',
    'oss.site': 'This website has no cookies, counters or third-party scripts either.',
    'oss.license': 'MPL-2.0 licence',
    'oss.roadmap': 'Roadmap',
    'oss.contribute': 'How to contribute',
    'donate.title': 'Support the project',
    'donate.text':
      'Kinolune lives on voluntary donations. Donation platforms are still being set up — the links will appear here. Bug reports, examples of wrong recognition and pull requests already help a lot.',
    'donate.issue': 'Report a bug',
    'donate.discuss': 'Discussions',
    'footer.license': 'Kinolune is free software under the MPL-2.0 licence.',
    'footer.trademarks': 'Names of third-party products and services belong to their owners.',
    'notfound.title': 'Page not found',
    'notfound.text': 'There is no such page. The link may be out of date.',
    'notfound.home': 'Home',
  },
  ru: {
    'meta.title': 'Kinolune — свободный медиаплеер для дома',
    'meta.description':
      'Фильмы, сериалы, аниме, музыка и Twitch на всех ваших устройствах — без облака и аккаунтов. Открытый код, MPL-2.0.',
    'nav.label': 'Разделы сайта',
    'nav.features': 'Возможности',
    'nav.install': 'Установить',
    'nav.opensource': 'Открытый код',
    'nav.donate': 'Поддержать',
    'nav.github': 'GitHub',
    'lang.label': 'Язык',
    'skip': 'Перейти к содержимому',
    'hero.badge': 'Альфа-версия',
    'hero.title': 'Ваша медиатека. Ваши устройства. Без облака.',
    'hero.lead':
      'Kinolune сам распознаёт фильмы, сериалы и аниме в ваших папках и сетевых дисках и воспроизводит почти любой формат — на компьютере, телефоне и телевизоре.',
    'hero.download': 'Скачать',
    'hero.source': 'Код на GitHub',
    'hero.note': 'Бесплатно и с открытым кодом. Без рекламы, аккаунтов и слежки.',
    'features.title': 'Возможности',
    'features.recognition.title': 'Распознаёт сам',
    'features.recognition.text':
      'Точно определяет фильмы, серии и аниме по именам файлов и папок. Постеры, описания и порядок серий — без ручной работы.',
    'features.playback.title': 'Играет почти всё',
    'features.playback.text':
      'Почти любые видео- и аудиоформаты, HDR, субтитры, передача звука на ресивер без перекодирования.',
    'features.sources.title': 'Откуда угодно',
    'features.sources.text':
      'Локальные папки, SMB, NFS, WebDAV, облачные диски, Plex, Jellyfin, Emby и собственный медиасервер.',
    'features.music.title': 'Музыка',
    'features.music.text':
      'Отдельный плеер: mp3, flac, alac, opus, dsd, воспроизведение без пауз между треками, тексты песен.',
    'features.twitch.title': 'Twitch',
    'features.twitch.text': 'Эфиры с чатом, записи, избранные стримеры — в одном приложении.',
    'features.devices.title': 'Все устройства, без облака',
    'features.devices.text':
      'Linux, Windows, macOS, Android и Android TV. Прогресс и медиатека синхронизируются в вашей домашней сети, а не через наши серверы.',
    'shots.title': 'Как это выглядит',
    'shots.desktop': 'Главный экран на компьютере',
    'shots.phone': 'Kinolune на телефоне',
    'shots.tv': 'Kinolune на телевизоре',
    'shots.note': 'На скриншотах — демо-медиатека с собственными обложками-заглушками.',
    'install.title': 'Установить',
    'install.lead':
      'Сборки для всех платформ — в GitHub Releases: Linux (.deb, .rpm, AppImage), Windows (установщик), macOS (.dmg) и Android (APK).',
    'install.releases': 'Все загрузки',
    'install.linux': 'Linux: из репозитория, с обновлениями вместе с системой',
    'install.apt': 'Ubuntu 24.04+, Debian 13+, Linux Mint 22+ (apt)',
    'install.dnf': 'Дистрибутивы с dnf (RPM)',
    'install.updates':
      'Дальше Kinolune обновляется вместе с системой (sudo apt upgrade, sudo dnf upgrade). Пакеты .deb и .rpm из релиза добавляют этот же репозиторий.',
    'install.copy': 'Копировать',
    'install.copied': 'Скопировано',
    'oss.title': 'Открытый код и приватность',
    'oss.text':
      'Весь код Kinolune открыт под лицензией MPL-2.0. Приложение работает локально: ваша медиатека, история просмотров и пароли остаются на ваших устройствах. У нас нет серверов, которые что-то о вас знают.',
    'oss.site': 'Этот сайт тоже без cookies, счётчиков и сторонних скриптов.',
    'oss.license': 'Лицензия MPL-2.0',
    'oss.roadmap': 'План разработки',
    'oss.contribute': 'Как участвовать',
    'donate.title': 'Поддержать проект',
    'donate.text':
      'Kinolune живёт на добровольные пожертвования. Площадки для пожертвований ещё подключаются — ссылки появятся здесь. Уже сейчас очень помогают отчёты об ошибках, примеры неверного распознавания и pull request’ы.',
    'donate.issue': 'Сообщить об ошибке',
    'donate.discuss': 'Обсуждения',
    'footer.license': 'Kinolune — свободное программное обеспечение под лицензией MPL-2.0.',
    'footer.trademarks': 'Названия сторонних продуктов и сервисов принадлежат их владельцам.',
    'notfound.title': 'Страница не найдена',
    'notfound.text': 'Такой страницы нет. Возможно, ссылка устарела.',
    'notfound.home': 'На главную',
  },
};

/** Translator for one locale; an unknown key throws at build time. */
export function useStrings(lang) {
  const dict = strings[lang];
  return (key) => {
    if (!(key in dict)) throw new Error(`site i18n: no "${key}" in ${lang}`);
    return dict[key];
  };
}

/** URL inside the site, respecting the configured base path. */
export function href(path = '') {
  const base = import.meta.env.BASE_URL.replace(/\/?$/, '/');
  return base + path.replace(/^\//, '');
}
