// SPDX-License-Identifier: MPL-2.0

package tui

import (
	"fmt"

	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
)

// Languages of the installer, Ukrainian first (the product is Ukrainian).
var Languages = []struct{ Code, Name string }{
	{"uk", "Українська"},
	{"en", "English"},
	{"ru", "Русский"},
}

func langIndex(code string) int {
	for i, l := range Languages {
		if l.Code == code {
			return i
		}
	}
	return 0
}

// texts: key -> {uk, en, ru}. Only plain characters that the Linux console
// font (LatArCyrHeb-16) has: no arrows, no box drawing, no ellipsis.
var texts = map[string][3]string{
	"app":  {"Встановлення", "Installer", "Установка"},
	"step": {"Крок %d з %d", "Step %d of %d", "Шаг %d из %d"},

	"lang.title": {"Мова / Language / Язык", "Мова / Language / Язык", "Мова / Language / Язык"},
	"lang.intro": {
		"Цей майстер встановить Kinolune OS на один диск цього комп'ютера.",
		"This wizard installs Kinolune OS onto one disk of this computer.",
		"Этот мастер установит Kinolune OS на один диск этого компьютера.",
	},

	"keys.list":    {"Вгору/Вниз: вибір  Enter: далі  Esc: назад  Ctrl+C: вихід", "Up/Down: select  Enter: next  Esc: back  Ctrl+C: exit", "Вверх/Вниз: выбор  Enter: далее  Esc: назад  Ctrl+C: выход"},
	"keys.disk":    {"Вгору/Вниз: диск  Enter: вибрати  R: оновити  Esc: назад  Ctrl+C: вихід", "Up/Down: disk  Enter: choose  R: rescan  Esc: back  Ctrl+C: exit", "Вверх/Вниз: диск  Enter: выбрать  R: обновить  Esc: назад  Ctrl+C: выход"},
	"keys.form":    {"Tab/Вниз: наступне поле  Пробіл: перемкнути  Enter: далі  Esc: назад", "Tab/Down: next field  Space: toggle  Enter: next  Esc: back", "Tab/Вниз: следующее поле  Пробел: переключить  Enter: далее  Esc: назад"},
	"keys.confirm": {"Введіть ім'я диска й натисніть Enter  Esc: назад  Ctrl+C: вихід", "Type the disk name and press Enter  Esc: back  Ctrl+C: exit", "Введите имя диска и нажмите Enter  Esc: назад  Ctrl+C: выход"},
	"keys.wait":    {"Зачекайте, не вимикайте комп'ютер", "Please wait, do not power off the computer", "Подождите, не выключайте компьютер"},
	"keys.menu":    {"Вгору/Вниз: вибір  Enter: виконати", "Up/Down: select  Enter: run", "Вверх/Вниз: выбор  Enter: выполнить"},

	"disk.title": {"Диск для системи", "System disk", "Диск для системы"},
	"disk.intro": {
		"Виберіть диск. УСЕ, ЩО НА НЬОМУ Є, БУДЕ СТЕРТО. Інші диски не змінюються.",
		"Choose a disk. EVERYTHING ON IT WILL BE ERASED. Other disks are not touched.",
		"Выберите диск. ВСЁ, ЧТО НА НЁМ ЕСТЬ, БУДЕТ СТЁРТО. Другие диски не меняются.",
	},
	"disk.none": {
		"Немає диска, на який можна встановити систему (потрібно щонайменше %s).",
		"There is no disk the system can be installed to (at least %s is needed).",
		"Нет диска, на который можно установить систему (нужно не меньше %s).",
	},
	"disk.empty":    {"диск порожній", "the disk looks empty", "диск пустой"},
	"disk.erase":    {"Буде стерто:", "Will be erased:", "Будет стёрто:"},
	"disk.more":     {"і ще %d", "and %d more", "и ещё %d"},
	"disk.excluded": {"Не пропонуються:", "Not offered:", "Не предлагаются:"},
	"disk.virtual":  {"віртуальні пристрої (loop, zram): %d", "virtual devices (loop, zram): %d", "виртуальные устройства (loop, zram): %d"},
	"disk.error":    {"Не вдалося прочитати диски: %s", "Cannot read the disks: %s", "Не удалось прочитать диски: %s"},

	"excl.boot-media":     {"з нього завантажено інсталятор", "the installer boots from it", "с него загружен установщик"},
	"excl.virtual-device": {"віртуальний пристрій", "virtual device", "виртуальное устройство"},
	"excl.optical":        {"оптичний привід", "optical drive", "оптический привод"},
	"excl.no-media":       {"немає носія", "no medium", "нет носителя"},
	"excl.read-only":      {"лише для читання", "read-only", "только для чтения"},
	"excl.too-small":      {"замалий (потрібно %s)", "too small (%s needed)", "слишком мал (нужно %s)"},
	"excl.hardware-area":  {"службова область eMMC", "eMMC hardware area", "служебная область eMMC"},
	"excl.not-a-disk":     {"не диск", "not a disk", "не диск"},
	"excl.mounted":        {"%s змонтовано в %s", "%s is mounted at %s", "%s смонтирован в %s"},

	"find.partition-table": {"таблиця розділів %s, розділів: %d", "%s partition table, %d partition(s)", "таблица разделов %s, разделов: %d"},
	"find.filesystem":      {"%s%s на %s", "%s%s on %s", "%s%s на %s"},
	"find.raid-member":     {"учасник RAID%s на %s", "RAID member%s on %s", "участник RAID%s на %s"},
	"find.lvm-member":      {"том LVM на %s", "LVM volume on %s", "том LVM на %s"},
	"find.luks":            {"зашифрований том LUKS на %s", "LUKS encrypted volume on %s", "зашифрованный том LUKS на %s"},
	"find.zfs-member":      {"пул ZFS%s на %s", "ZFS pool%s on %s", "пул ZFS%s на %s"},
	"find.swap":            {"swap на %s", "swap on %s", "swap на %s"},
	"find.os-windows":      {"встановлена Windows (%s)", "Windows is installed (%s)", "установлена Windows (%s)"},
	"find.os-efi":          {"завантажувальний розділ іншої ОС (%s)", "boot partition of another OS (%s)", "загрузочный раздел другой ОС (%s)"},
	"find.mounted":         {"%s змонтовано в %s", "%s is mounted at %s", "%s смонтирован в %s"},
	"find.smart-failed":    {"SMART: диск несправний, не ставте на нього систему", "SMART: the disk is failing, do not install onto it", "SMART: диск неисправен, не ставьте на него систему"},

	"layout.title": {"Розмітка диска", "Disk layout", "Разметка диска"},
	"layout.intro": {
		"За замовчуванням увесь диск віддається системі. Можна залишити решту диска під дані.",
		"By default the whole disk goes to the system. You can keep the rest of it for data.",
		"По умолчанию весь диск отдаётся системе. Можно оставить остаток диска под данные.",
	},
	"layout.data":     {"Залишити решту диска під дані", "Keep the rest of the disk for data", "Оставить остаток диска под данные"},
	"layout.root":     {"Розмір системи, ГіБ", "System size, GiB", "Размер системы, ГиБ"},
	"layout.pool":     {"Ім'я сховища даних", "Data pool name", "Имя хранилища данных"},
	"layout.whole":    {"Система: увесь диск (%s)", "System: the whole disk (%s)", "Система: весь диск (%s)"},
	"layout.split":    {"Система: %d ГіБ, дані: решта (%s) у /var/srv/data/%s", "System: %d GiB, data: the rest (%s) in /var/srv/data/%s", "Система: %d ГиБ, данные: остаток (%s) в /var/srv/data/%s"},
	"layout.luks":     {"Шифрування диска (LUKS) з'явиться в наступній версії.", "Disk encryption (LUKS) comes in a later version.", "Шифрование диска (LUKS) появится в следующей версии."},
	"err.root_size":   {"Розмір системи: ціле число ГіБ, щонайменше %d.", "System size: a whole number of GiB, at least %d.", "Размер системы: целое число ГиБ, не меньше %d."},
	"err.data_pool":   {"Ім'я сховища: малі латинські літери, цифри, - і _.", "Pool name: lower-case letters, digits, - and _.", "Имя хранилища: строчные латинские буквы, цифры, - и _."},
	"err.layout":      {"Не вміщається на диск: %s", "Does not fit the disk: %s", "Не помещается на диск: %s"},
	"host.title":      {"Ім'я комп'ютера", "Computer name", "Имя компьютера"},
	"host.intro":      {"Під цим ім'ям сервер видно в мережі: https://<ім'я>.local", "The server is seen on the network under this name: https://<name>.local", "Под этим именем сервер виден в сети: https://<имя>.local"},
	"host.name":       {"Ім'я хоста", "Host name", "Имя хоста"},
	"err.hostname":    {"Ім'я хоста: малі латинські літери, цифри та -, не довше 63 символів.", "Host name: lower-case letters, digits and -, up to 63 characters.", "Имя хоста: строчные латинские буквы, цифры и -, не длиннее 63 символов."},
	"user.title":      {"Адміністратор", "Administrator", "Администратор"},
	"user.intro":      {"Цим обліковим записом ви входите в консоль і в панель керування.", "You sign in to the console and the admin panel with this account.", "С этой учётной записью вы входите в консоль и в панель управления."},
	"user.name":       {"Ім'я користувача", "User name", "Имя пользователя"},
	"user.full":       {"Повне ім'я (необов'язково)", "Full name (optional)", "Полное имя (необязательно)"},
	"user.pw":         {"Пароль", "Password", "Пароль"},
	"user.pw2":        {"Пароль ще раз", "Password again", "Пароль ещё раз"},
	"user.keep":       {"Порожній пароль: залишиться пароль із файлу відповідей.", "Empty password: the one from the answer file is kept.", "Пустой пароль: останется пароль из файла ответов."},
	"err.user":        {"Ім'я: малі латинські літери, цифри, - і _, з літери; не системне (root, wheel...).", "Name: lower-case letters, digits, - and _, starting with a letter; not a system name (root, wheel...).", "Имя: строчные латинские буквы, цифры, - и _, с буквы; не системное (root, wheel...)."},
	"err.full":        {"Повне ім'я не може містити ':'.", "The full name must not contain ':'.", "Полное имя не может содержать ':'."},
	"err.pw_short":    {"Пароль: щонайменше %d символів.", "Password: at least %d characters.", "Пароль: не меньше %d символов."},
	"err.pw_mismatch": {"Паролі не збігаються.", "The passwords do not match.", "Пароли не совпадают."},

	"net.title":    {"Мережа", "Network", "Сеть"},
	"net.intro":    {"Зазвичай адресу видає роутер (DHCP). Статична адреса - для досвідчених.", "Usually the router gives the address (DHCP). A static address is for experts.", "Обычно адрес выдаёт роутер (DHCP). Статический адрес - для опытных."},
	"net.mode":     {"Налаштування", "Configuration", "Настройка"},
	"net.dhcp":     {"Автоматично (DHCP)", "Automatic (DHCP)", "Автоматически (DHCP)"},
	"net.static":   {"Статична адреса", "Static address", "Статический адрес"},
	"net.iface":    {"Мережевий порт", "Network port", "Сетевой порт"},
	"net.any":      {"будь-який дротовий", "any wired", "любой проводной"},
	"net.cable":    {"кабель підключено", "cable connected", "кабель подключён"},
	"net.nocable":  {"без кабелю", "no cable", "без кабеля"},
	"net.address":  {"Адреса/префікс", "Address/prefix", "Адрес/префикс"},
	"net.gateway":  {"Шлюз", "Gateway", "Шлюз"},
	"net.dns":      {"DNS (через кому)", "DNS (comma-separated)", "DNS (через запятую)"},
	"err.address":  {"Адреса: IPv4 із префіксом, наприклад 192.168.1.20/24.", "Address: IPv4 with a prefix, e.g. 192.168.1.20/24.", "Адрес: IPv4 с префиксом, например 192.168.1.20/24."},
	"err.gateway":  {"Шлюз: IPv4-адреса в тій самій мережі, наприклад 192.168.1.1.", "Gateway: an IPv4 address in the same network, e.g. 192.168.1.1.", "Шлюз: IPv4-адрес в той же сети, например 192.168.1.1."},
	"err.dns":      {"DNS: щонайменше одна IP-адреса.", "DNS: at least one IP address.", "DNS: хотя бы один IP-адрес."},
	"err.network":  {"Мережа: %s", "Network: %s", "Сеть: %s"},
	"sum.title":    {"Перевірте й підтвердьте", "Check and confirm", "Проверьте и подтвердите"},
	"sum.disk":     {"Диск", "Disk", "Диск"},
	"sum.layout":   {"Розмітка", "Layout", "Разметка"},
	"sum.host":     {"Ім'я", "Name", "Имя"},
	"sum.user":     {"Адмін", "Admin", "Админ"},
	"sum.net":      {"Мережа", "Network", "Сеть"},
	"sum.updates":  {"Оновлення", "Updates", "Обновления"},
	"sum.channel":  {"канал %s, з %s", "channel %s, from %s", "канал %s, из %s"},
	"sum.lang":     {"Мова", "Language", "Язык"},
	"sum.static":   {"%s, шлюз %s, DNS %s", "%s, gateway %s, DNS %s", "%s, шлюз %s, DNS %s"},
	"sum.on":       {"порт %s", "port %s", "порт %s"},
	"sum.wipe":     {"УВАГА: усі дані на %s буде знищено без можливості відновлення.", "WARNING: all data on %s will be destroyed and cannot be recovered.", "ВНИМАНИЕ: все данные на %s будут уничтожены без возможности восстановления."},
	"sum.confirm":  {"Щоб почати, введіть ім'я диска %s:", "To start, type the disk name %s:", "Чтобы начать, введите имя диска %s:"},
	"sum.dryrun":   {"Пробний запуск: на диск нічого не записується.", "Dry run: nothing is written to the disk.", "Пробный запуск: на диск ничего не записывается."},
	"err.confirm":  {"Ім'я не збігається. Введіть %s або натисніть Esc.", "The name does not match. Type %s or press Esc.", "Имя не совпадает. Введите %s или нажмите Esc."},
	"err.invalid":  {"Відповіді не пройшли перевірку: %s", "The answers are not valid: %s", "Ответы не прошли проверку: %s"},
	"prog.title":   {"Встановлення Kinolune OS", "Installing Kinolune OS", "Установка Kinolune OS"},
	"prog.dry":     {"Пробний запуск: план встановлення", "Dry run: installation plan", "Пробный запуск: план установки"},
	"prog.elapsed": {"Минуло %s", "Elapsed %s", "Прошло %s"},
	"prog.log":     {"Журнал: %s", "Log: %s", "Журнал: %s"},
	"prog.busy":    {"Йде встановлення, вихід недоступний.", "The installation is running, exit is not available.", "Идёт установка, выход недоступен."},
	"done.title":   {"Kinolune OS встановлено", "Kinolune OS is installed", "Kinolune OS установлена"},
	"done.dry":     {"Пробний запуск завершено", "Dry run finished", "Пробный запуск завершён"},
	"done.dry1":    {"Файл відповідей правильний, диск не змінювався.", "The answers are valid, the disk was not changed.", "Ответы правильные, диск не менялся."},
	"done.next":    {"Що далі:", "Next steps:", "Что дальше:"},
	"done.n1":      {"1. Вийміть флешку інсталятора й перезавантажте комп'ютер.", "1. Remove the installer USB stick and reboot the computer.", "1. Извлеките флешку установщика и перезагрузите компьютер."},
	"done.n2":      {"2. Панель керування: %s у браузері в тій самій мережі.", "2. Admin panel: %s in a browser on the same network.", "2. Панель управления: %s в браузере в той же сети."},
	"done.n2b":     {"Якщо адреса .local не відкривається, IP сервера є в списку пристроїв роутера.", "If the .local address does not open, find the server IP in the router's device list.", "Если адрес .local не открывается, IP сервера есть в списке устройств роутера."},
	"done.n3":      {"3. Вхід: користувач %s і пароль, який ви задали.", "3. Sign in as %s with the password you set.", "3. Вход: пользователь %s и пароль, который вы задали."},
	"fail.title":   {"Встановлення не вдалося", "The installation failed", "Установка не удалась"},
	"fail.partial": {"Диск міг бути записаний частково. Інші диски не змінювалися.", "The disk may be partly written. Other disks were not touched.", "Диск мог быть записан частично. Другие диски не менялись."},
	"act.reboot":   {"Перезавантажити", "Reboot", "Перезагрузить"},
	"act.poweroff": {"Вимкнути", "Power off", "Выключить"},
	"act.save":     {"Зберегти файл відповідей для повторних встановлень", "Save an answer file for repeat installs", "Сохранить файл ответов для повторных установок"},
	"act.menu":     {"Текстове меню й командний рядок", "Text menu and command line", "Текстовое меню и командная строка"},
	"act.back":     {"Назад до підтвердження", "Back to the confirmation", "Назад к подтверждению"},
	"act.continue": {"Продовжити встановлення", "Continue the installation", "Продолжить установку"},
	"quit.title":   {"Вийти з інсталятора?", "Leave the installer?", "Выйти из установщика?"},
	"save.title":   {"Зберегти файл відповідей", "Save an answer file", "Сохранить файл ответов"},
	"save.intro": {
		"Файл kinolune-answers.json дозволяє повторити встановлення: текстове меню знайде його на флешці.",
		"kinolune-answers.json repeats this installation: the text menu finds it on a USB stick.",
		"Файл kinolune-answers.json позволяет повторить установку: текстовое меню найдёт его на флешке.",
	},
	"save.hash":   {"Пароль зберігається лише як хеш yescrypt.", "The password is stored only as a yescrypt hash.", "Пароль хранится только как хеш yescrypt."},
	"save.plain":  {"УВАГА: пароль буде у файлі відкритим текстом. Видаліть файл після використання.", "WARNING: the password is stored in clear text. Delete the file after use.", "ВНИМАНИЕ: пароль будет в файле открытым текстом. Удалите файл после использования."},
	"save.local":  {"У live-системі: %s (зникне після перезавантаження)", "In the live system: %s (gone after reboot)", "В live-системе: %s (пропадёт после перезагрузки)"},
	"save.none":   {"Підключіть флешку з FAT, exFAT або ext4 і натисніть R.", "Plug in a USB stick with FAT, exFAT or ext4 and press R.", "Подключите флешку с FAT, exFAT или ext4 и нажмите R."},
	"save.ok":     {"Збережено: %s", "Saved: %s", "Сохранено: %s"},
	"save.failed": {"Не вдалося зберегти: %s", "Cannot save: %s", "Не удалось сохранить: %s"},
	"keys.save":   {"Вгору/Вниз: куди  Enter: зберегти  R: оновити  Esc: назад", "Up/Down: where  Enter: save  R: rescan  Esc: back", "Вверх/Вниз: куда  Enter: сохранить  R: обновить  Esc: назад"},
}

// tr translates key into lang (index into Languages), formatting args.
func tr(lang int, key string, args ...any) string {
	t, ok := texts[key]
	if !ok {
		return key
	}
	s := t[lang]
	if s == "" {
		s = t[1]
	}
	if len(args) > 0 {
		return fmt.Sprintf(s, args...)
	}
	return s
}

// findingText localises a disk finding (disks.Finding.String is the English fallback).
func findingText(lang int, f disks.Finding) string {
	label := ""
	if f.Label != "" {
		label = fmt.Sprintf(" %q", f.Label)
	}
	switch f.Kind {
	case disks.FindingPartitionTable:
		return tr(lang, "find.partition-table", upper(f.Detail), f.Count)
	case disks.FindingFilesystem:
		return tr(lang, "find.filesystem", f.FSType, label, f.Device)
	case disks.FindingRAIDMember, disks.FindingZFSMember:
		return tr(lang, "find."+string(f.Kind), label, f.Device)
	case disks.FindingLVMMember, disks.FindingLUKS, disks.FindingSwap, disks.FindingWindows, disks.FindingEFISystem:
		return tr(lang, "find."+string(f.Kind), f.Device)
	case disks.FindingMounted:
		return tr(lang, "find.mounted", f.Device, f.Detail)
	case disks.FindingSmartFailed:
		return tr(lang, "find.smart-failed")
	}
	return f.String()
}

// excludeText localises why a disk is not offered.
func excludeText(lang int, d disks.Disk) string {
	if d.ExcludeReason == disks.ExcludeTooSmall {
		return tr(lang, "excl.too-small", disks.FormatDecimal(disks.DefaultMinSize))
	}
	if _, ok := texts["excl."+string(d.ExcludeReason)]; ok {
		return tr(lang, "excl."+string(d.ExcludeReason))
	}
	return string(d.ExcludeReason)
}

func upper(s string) string {
	b := []byte(s)
	for i, c := range b {
		if c >= 'a' && c <= 'z' {
			b[i] = c - 32
		}
	}
	return string(b)
}
