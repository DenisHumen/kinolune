<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: Wi-Fi и сеть с главной: NetworkManager по D-Bus из Dart, polkit-правило образа

- **Статус:** Предложен
- **Дата:** 2026-11-25
- **Решают:** мейнтейнер (владелец делегировал решения)
- **Связано:** DIST-032, DIST-001, DIST-029, DIST-031, UI-027; [ADR-0010](0010-nas-os-phase0.md), [ADR-0007](0007-store-full-build-flavors.md)

## Контекст и проблема

Запрос владельца 2026-11-18. На мини-ПК с Kinolune OS Wi-Fi должен настраиваться прямо из Kinolune, без рабочего стола. На главной — значок сети: сигнал, «нет сети», «вход в сеть» для captive portal. По нажатию — панель: сети с сигналом и замком, подключение с паролем и без, скрытая сеть, «Забыть», автоподключение, кабель, IP и «Проверить интернет». Панель работает и на любом Linux-десктопе с NetworkManager. На Android, iOS, macOS и Windows её нет — там системные настройки.

Ограничения:

- без root;
- пароли — только в системе: не в нашей БД, не в логах, не в синхронизации;
- лицензии — MPL, MIT, BSD или Apache;
- сейчас главный фокус — десктоп и дистрибутив.

## Рассмотренные варианты

1. **Dart-пакет `dbus` (Canonical, MPL-2.0), свой тонкий клиент NetworkManager.** Ни нативного кода, ни FRB. Пакет умеет поднимать свою шину (`DBusServer`), поэтому в тестах можно запустить поддельный NetworkManager на настоящем D-Bus.
2. **Пакет `nm` (Canonical, MPL-2.0) поверх `dbus`.** Готовые обёртки, но объём API намного больше нужного. Поддельный NetworkManager для тестов тогда пришлось бы повторять во всю ширину того, что пакет читает при старте.
3. **Модуль `zbus` в Rust-ядре через FRB.** Код ядра можно было бы переиспользовать в будущих демонах дистрибутива. Но нужна перегенерация FRB, растёт pa-frb, а ядро тянуло бы D-Bus, который нужен только Linux-клиенту.
4. **Вызывать `nmcli`.** Пришлось бы разбирать текст. Пароль попал бы в аргументы процесса, а их видно в `ps`.

## Решение

Выбран вариант **1**.

- **Клиент.** `lib/network/nm_backend.dart` работает с системной шиной и использует только стабильный API NetworkManager ≥ 1.20:
  - корень: `Devices`, `WirelessEnabled`, `Connectivity`, `ConnectivityCheckUri`, `CheckConnectivity`, `ActivateConnection`, `AddAndActivateConnection`;
  - `Device`, `Device.Wireless` (`RequestScan`), `Device.Wired` (`Carrier`);
  - `AccessPoint`, `IP4Config`;
  - `Settings` и `Settings.Connection` (`GetSettings`, `Update`, `Delete`).
- **Интерфейс.** `NetworkBackend` (`network_backend.dart`) отделяет UI от системы. В демо-режиме и в widget-тестах работает `FakeNetworkBackend`.
- **Когда панель видна.**
  - Linux, NetworkManager отвечает и есть управляемый Wi-Fi-адаптер — видна.
  - Иначе `NetworkSnapshot.unavailable`, значок не рисуется.
  - На других ОС контроллер не создаётся вовсе (`openNetwork`).
- **Пароль.**
  - Уходит один раз: в `AddAndActivateConnection` (новая сеть) или в `Update` (смена пароля сохранённой сети).
  - Если подключение не удалось, только что созданный профиль удаляется. Неверный пароль не остаётся даже в NetworkManager.
  - `GetSettings` секретов не возвращает. Автоподключение меняется через `Update` без секретов, и NetworkManager сохраняет старые.
  - У нас пароль не пишется никуда. На ТВ поле пароля — обычное скрытое поле, его можно набрать с телефона (UI-027, запечатанным).
- **Неверный пароль.** Причина отказа устройства `NO_SECRETS` (7) или `SUPPLICANT_DISCONNECT` (8) при заданном пароле. Пароль WPA короче 8 символов NetworkManager отвергает сразу (`InvalidProperty`).
- **Captive portal.** Состояние `Connectivity = PORTAL`. «Открыть страницу входа» открывает адрес проверки, который задан в самой системе (`ConnectivityCheckUri`). Своих адресов нет.
- **802.1X (корпоративные сети)** — только в настройках системы. Сеть показана, но панель её не подключает.
- **Kinolune OS.**
  - В образе есть `NetworkManager`, `NetworkManager-wifi`, `wpa_supplicant` и `polkit`.
  - Правило `/usr/share/polkit-1/rules.d/50-kinolune-network.rules` разрешает группе `kinolune` только нужные панели действия: `network-control`, `enable-disable-wifi`, `wifi.scan`, `settings.modify.own`, `settings.modify.system`.
  - Группа создаётся через `sysusers.d`. Пользователь Kinolune-сессии (DIST-029) входит в неё.
  - На обычных десктопах хватает штатных правил NetworkManager для активной локальной сессии.
- **Обновление состояния.** Сигналы `PropertiesChanged` NetworkManager, плюс опрос раз в 20 с (с открытой панелью — раз в 4 с).

## Последствия

- **Плюсы:**
  - Ни нативного кода, ни изменений в ядре и FRB.
  - Клиент проверяется против поддельного NetworkManager на настоящем D-Bus (`test/support/fake_network_manager.dart`), без системной шины.
  - Тот же код собирается в `tool/nm_probe.dart` для живой проверки с `mac80211_hwsim` (`distro/tests/wifi-hwsim*.sh`).
- **Минусы:**
  - Логика NetworkManager живёт в Dart. Будущим демонам дистрибутива на Go (ADR-0010) понадобится своя.
  - WEP поддержан лишь базово. 802.1X нет.
- **Для DIST-031** (системные экраны оболочки): экран сети берётся отсюда (`NetworkPanel`), свой не делается.

## Проверка

- `test/network_nm_test.dart` — клиент против поддельного NetworkManager.
- `test/network_panel_test.dart` — компьютер, телефон и ТВ, ru/en/uk.
- `distro/tests/wifi-hwsim-vm.sh` — VM с Fedora Cloud, NetworkManager, hostapd и `mac80211_hwsim`: точки WPA2 и открытая, пользователь из группы `kinolune` без root.
