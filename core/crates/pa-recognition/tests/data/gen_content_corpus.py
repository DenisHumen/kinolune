#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
# LIB-025: generator of the synthetic content-kind corpus. Writes JSONL to argv[1].
# Every path is made up; titles of published works are public, personal clips
# get invented names. Deterministic (seeded).
import json, random, sys

rnd = random.Random(25)
rows = []

def add(kind, path, minutes=None, tags=None, genres=None, texts=None, chapters=0, note=None):
    r = {"path": path, "kind": kind}
    if minutes is not None:
        r["duration_ms"] = int(minutes * 60_000)
    if tags:
        r["tags"] = tags
    if genres:
        r["genres"] = genres
    if texts:
        r["texts"] = texts
    if chapters:
        r["chapters"] = chapters
    if note:
        r["note"] = note
    rows.append(r)

mux = lambda: {"creation_time": "2019-03-0%dT10:00:00Z" % rnd.randint(1, 9), "encoder": rnd.choice(["libebml v1.4.2 + libmatroska v1.6.4", "Lavf58.76.100", "HandBrake 1.6.1"])}

# --- Movies (≈70) -------------------------------------------------------------
movies = [("Inception", 2010), ("Arrival", 2016), ("Heat", 1995), ("Alien", 1979), ("Brazil", 1985),
          ("Gattaca", 1997), ("Amélie", 2001), ("Oldboy", 2003), ("Parasite", 2019), ("Dune", 2021),
          ("Up", 2009), ("Her", 2013), ("Drive", 2011), ("Moon", 2009), ("Coco", 2017),
          ("Брат", 1997), ("Сталкер", 1979), ("Левиафан", 2014), ("Солярис", 1972), ("Морфий", 2008),
          ("The Matrix", 1999), ("No Country for Old Men", 2007), ("Spirited Away", 2001), ("The Thing", 1982),
          ("Blade Runner 2049", 2017), ("Mad Max Fury Road", 2015)]
for i, (t, y) in enumerate(movies):
    dot = t.replace(" ", ".")
    style = i % 7
    if style == 0:
        add("video", f"Movies/{t} ({y})/{t} ({y}).mkv", 120, mux())
    elif style == 1:
        add("video", f"Movies/{dot}.{y}.1080p.BluRay.x264-GROUP.mkv", 118)
    elif style == 2:
        add("video", f"Фильмы/{t} ({y}).avi", 110)
    elif style == 3:
        add("video", f"Кино/{t} {y} WEB-DL 720p.mkv", 125, mux())
    elif style == 4:
        add("video", f"Movies/{t} ({y})/{dot}.{y}.2160p.UHD.BluRay.REMUX.HDR.HEVC-GRP.mkv", 130)
        add("video", f"Movies/{t} ({y})/Featurettes/Making of.mkv", 12)
    elif style == 5:
        add("video", f"Media/Films/{t} [{y}].mp4", 105)
    else:
        add("video", f"Movies/{t}/{t}.{y}.DVDRip.XviD.avi", 100)
# Movies whose names say nothing but the title (no year, no release tags).
for t in ["Inception", "Amelie", "Леон", "Брат 2", "Interstellar", "Gladiator", "Титаник", "Shrek",
          "Up", "Avatar"]:
    add("video", f"Movies/{t}.mkv", 120)
for t in ["Collector's Edition/Disc 1/VIDEO_TS/VTS_01_1.VOB", "BDMV/STREAM/00000.m2ts"]:
    add("video", f"Movies/Some Film (2004)/{t}", 90)
add("video", "Movies/Pi (1998)/Pi.1998.720p.BluRay.mkv", 84)
add("video", "Movies/1917 (2019)/1917.2019.1080p.WEB-DL.mkv", 119)
add("video", "Movies/2001 A Space Odyssey (1968).mkv", 149)
add("video", "Movies/Video Nasty (2021).mkv", 90)
add("video", "Movies/The Camera Roll (2018)/The.Camera.Roll.2018.1080p.mkv", 95)
add("video", "Movies/Clip (2012).mkv", 101)
add("video", "Films/Сага/Часть 1.mkv", 110, note="no year, no tags")
add("video", "Films/Сага/Часть 2.mkv", 115)

# --- Shows (≈70) ---------------------------------------------------------------
shows = ["Dark", "Fargo", "Chernobyl", "The Office", "Severance", "Метод", "Шерлок", "Слово пацана",
         "Breaking Bad", "Mr Robot"]
for i, s in enumerate(shows):
    dot = s.replace(" ", ".")
    for e in range(1, 6):
        style = i % 5
        if style == 0:
            add("video", f"Shows/{s}/Season 1/{dot}.S01E{e:02}.1080p.WEB.h264-GRP.mkv", 50)
        elif style == 1:
            add("video", f"Сериалы/{s}/Сезон 1/{s} - 1x{e:02}.mkv", 45)
        elif style == 2:
            add("video", f"TV/{s}/Season 01/{e:02} - Episode {e}.mkv", 55, mux())
        elif style == 3:
            add("video", f"Сериалы/{s}/Сезон 2/Серия {e}.avi", 48)
        else:
            add("video", f"TV/{s} (2015)/{s} S02E{e:02}.mp4", 42)
for d in range(10, 16):
    add("video", f"TV/The Daily Show/The Daily Show 2024-06-{d} Guest Name.mkv", 22)
for e in range(1, 5):
    add("video", f"TV/Nature Docs/Planet Earth II/Episode {e} - Islands.mkv", 58)

# --- Anime (≈40) ---------------------------------------------------------------
anime = ["Frieren", "Spy x Family", "Mushishi", "Bocchi the Rock", "Made in Abyss", "Monster", "Vinland Saga", "Trigun"]
for i, a in enumerate(anime):
    for e in range(1, 6):
        style = i % 4
        if style == 0:
            add("video", f"Anime/{a}/[SubsPlease] {a} - {e:02} (1080p) [ABCD{e:04}].mkv", 24)
        elif style == 1:
            add("video", f"Anime/[Erai-raws] {a} - {e:02} [720p][Multiple Subtitle].mkv", 24)
        elif style == 2:
            add("video", f"Аниме/{a}/{a} - {e:03}.mkv", 24)
        else:
            add("video", f"Anime/{a} (2003)/{a} - S01E{e:02} - Title.mkv", 24)

# --- Albums (≈90 tracks) -------------------------------------------------------
albums = [
    ("Music/Radiohead/1997 - OK Computer", ["Alternative Rock"], 12, (3, 6)),
    ("Music/Miles Davis/1959 - Kind of Blue [FLAC]", ["Jazz"], 5, (5, 11)),
    ("Music/Кино/1988 - Группа крови", ["Rock"], 11, (3, 5)),
    ("Music/Daft Punk/2013 - Random Access Memories (24bit)", ["Electronic"], 13, (4, 9)),
    ("Music/Various Artists/Now That's What I Call Music 100 (CD1)", [], 20, (3, 4)),
    ("Music/Pink Floyd/1975 - Wish You Were Here", ["Progressive Rock"], 5, (5, 14)),
    ("Music/Beethoven/Symphony No. 9 - Karajan", ["Classical"], 4, (12, 25)),
    ("Музыка/Сплин/2001 - 25-й кадр", [], 12, (3, 5)),
    ("Music/Max Richter/2015 - Sleep (Excerpts)", ["Classical", "Ambient"], 6, (8, 30)),
    ("Music/DJ Sets/Essential Mix 2019", ["Electronic"], 2, (55, 120)),
    ("Music/Podcast-free/Lo-fi Beats Vol 3", ["Lo-Fi"], 10, (2, 3)),
    ("Music/Yes/1972 - Close to the Edge", [], 3, (9, 19)),
]
for folder, genres, n, (lo, hi) in albums:
    for t in range(1, n + 1):
        title = rnd.choice(["Song", "Paranoid Android", "So What", "Part I", "Interlude", "Movement", "Track", "Без названия", "Chapter Two", "Lucky"])
        ext = "flac" if "FLAC" in folder or "24bit" in folder or "Classical" in genres else rnd.choice(["mp3", "m4a", "flac", "opus"])
        add("music", f"{folder}/{t:02} - {title}.{ext}", rnd.uniform(lo, hi), genres=genres,
            texts=[title, folder.rsplit('/', 1)[-1]])
for i in range(1, 9):
    add("music", f"Downloads/music/Artist {i} - Single {i}.mp3", rnd.uniform(2.5, 4.5))
add("music", "Music/Steve Reich/Music for 18 Musicians.flac", 62, genres=["Minimalism"],
    note="one long piece, music")
add("music", "Music/Tool/2019 - Fear Inoculum/10 - 7empest.flac", 15.7, genres=["Metal"])

# --- Audiobooks (≈90 tracks) ---------------------------------------------------
add("audiobook", "Audiobooks/Frank Herbert - Dune.m4b", 1260, chapters=48)
add("audiobook", "Audiobooks/Andy Weir/Project Hail Mary.m4b", 970, chapters=35)
add("audiobook", "Books/Neil Gaiman - Norse Mythology (narrated by the author).m4a", 390, chapters=16)
add("audiobook", "Books/Ursula K. Le Guin - A Wizard of Earthsea.m4a", 500, chapters=10)
for i in range(1, 31):
    add("audiobook", f"Аудиокниги/Булгаков - Мастер и Маргарита/{i:02}.mp3", rnd.uniform(9, 15))
for i in range(1, 13):
    add("audiobook", f"Книги/Стругацкие - Трудно быть богом (читает Кузнецов)/Глава {i:02}.mp3", rnd.uniform(15, 35))
for i in range(1, 11):
    add("audiobook", f"Audio/The Hobbit/Chapter {i:02}.mp3", rnd.uniform(25, 50))
for i in range(1, 9):
    add("audiobook", f"Audio/Sapiens/Sapiens - Part {i}.mp3", rnd.uniform(60, 90),
        genres=["Audiobook"], texts=["Sapiens", "Yuval Noah Harari"])
for i in range(1, 7):
    add("audiobook", f"Library/Чехов - Рассказы/{i:02} - Рассказ.mp3", rnd.uniform(18, 40),
        genres=["Speech"], texts=["Рассказы", "А. П. Чехов"])
for i in range(1, 5):
    add("audiobook", f"Радио/Радиоспектакль - Двенадцать стульев/{i}.mp3", rnd.uniform(40, 60))
for i in range(1, 7):
    add("audiobook", f"Audio/Pratchett - Guards! Guards!/Track {i:02}.mp3", rnd.uniform(55, 75),
        texts=["Guards! Guards!", "Read by Nigel Planer"])
# Hard: a book ripped into short tracks with no tag and no word in the path.
for i in range(1, 9):
    add("audiobook", f"Downloads/Author Name - Some Novel/{i:02}.mp3", rnd.uniform(5, 8),
        note="hard: short tracks, no signal")
for i in range(1, 4):
    add("audiobook", f"Audio/Курс английского/Урок {i}.mp3", rnd.uniform(20, 30),
        genres=["Speech"], note="spoken course")

# --- Phone and camera clips (≈80) ----------------------------------------------
def ts(i):
    return f"2024{rnd.randint(1, 12):02}{rnd.randint(1, 28):02}_{rnd.randint(0, 23):02}{rnd.randint(0, 59):02}{rnd.randint(0, 59):02}"
for i in range(10):
    add("own_video", f"Phone/DCIM/Camera/VID_{ts(i)}.mp4", rnd.uniform(0.2, 4), {"com.android.version": "14", "creation_time": "2024-06-12T18:30:00Z"})
for i in range(8):
    add("own_video", f"iPhone/IMG_{rnd.randint(1000, 9999)}.MOV", rnd.uniform(0.1, 3), {"com.apple.quicktime.make": "Apple", "com.apple.quicktime.model": "iPhone 13"})
for i in range(5):
    add("own_video", f"Видео/С телефона/{ts(i)}.mp4", rnd.uniform(0.2, 5))
for i in range(5):
    add("own_video", f"Pixel/PXL_{ts(i)}{rnd.randint(100, 999)}.mp4", rnd.uniform(0.2, 2))
for i in range(4):
    add("own_video", f"GoPro/2023 Alps/GX01{rnd.randint(1000, 9999)}.MP4", rnd.uniform(1, 12))
for i in range(3):
    add("own_video", f"GoPro/GOPR{rnd.randint(1000, 9999)}.MP4", rnd.uniform(1, 12))
for i in range(4):
    add("own_video", f"WhatsApp/Media/WhatsApp Video/VID-2024{rnd.randint(1, 12):02}{rnd.randint(10, 28)}-WA00{rnd.randint(10, 99)}.mp4", rnd.uniform(0.1, 2))
for i in range(3):
    add("own_video", f"Drone/DJI_{rnd.randint(1, 999):04}.MP4", rnd.uniform(1, 8))
for i in range(3):
    add("own_video", f"Camera Uploads/2024-0{i + 1}-1{i} 1{i}.2{i}.3{i}.mov", rnd.uniform(0.1, 2))
for i in range(3):
    add("own_video", f"Desktop/Screen Recording 2024-06-1{i} at 18.30.0{i}.mov", rnd.uniform(1, 10))
for i in range(3):
    add("own_video", f"Sony/PRIVATE/M4ROOT/CLIP/C{rnd.randint(1, 999):04}.MP4", rnd.uniform(1, 10))
for i in range(2):
    add("own_video", f"Canon/DCIM/100CANON/MVI_{rnd.randint(1000, 9999)}.MOV", rnd.uniform(1, 5))
for i in range(2):
    add("own_video", f"Panasonic/P10{rnd.randint(10000, 99999)}.MP4", rnd.uniform(1, 5))
for i in range(3):
    add("own_video", f"Dashcam/2024_06{10 + i}_183000_00{i}.MP4", 3)
# Own clips with names the user gave, phone tags kept.
for n in ["Море", "Первые шаги", "Выпускной", "Birthday cake", "Concert front row"]:
    add("own_video", f"Отпуск 2023/{n}.mp4", rnd.uniform(0.5, 6), {"com.apple.quicktime.make": "Apple", "com.apple.quicktime.model": "iPhone 12"})
for n in ["Свадьба", "Утренник в саду"]:
    add("own_video", f"Семейное/{n}.mp4", rnd.uniform(5, 40), {"make": "Samsung", "model": "SM-G991B"})
# A trip folder: some clips renamed, most not.
for i in range(4):
    add("own_video", f"Trips/Japan 2019/VID_2019{rnd.randint(3, 4):02}{rnd.randint(10, 28)}_{rnd.randint(100000, 235959)}.mp4", rnd.uniform(0.2, 3))
add("own_video", "Trips/Japan 2019/Kyoto temple.mp4", 2)
add("own_video", "Trips/Japan 2019/Shibuya crossing.mp4", 1)
# Hard: exported without tags, in a plain folder.
add("own_video", "Видео/Свадьба Маши.mp4", 35, note="hard: edited export, no tags")
add("own_video", "Videos/Graduation speech.mp4", 12, note="hard: edited export, no tags")

# --- Mixed folders (≈30) -------------------------------------------------------
add("video", "Downloads/Arrival.2016.1080p.WEB-DL.DD5.1.H264-GRP.mkv", 116)
add("own_video", "Downloads/VID_20240612_183000.mp4", 1, {"com.android.version": "13"})
add("own_video", "Downloads/IMG_0042.MOV", 0.5)
add("music", "Downloads/Artist - Song (Official Audio).mp3", 3.5)
add("audiobook", "Downloads/Stephen King - The Outsider (Unabridged).m4b", 1100, chapters=60)
add("video", "Downloads/Severance.S02E03.720p.WEB.mkv", 50)
add("video", "Telegram Desktop/Oppenheimer.2023.1080p.mkv", 180)
add("own_video", "Telegram Desktop/video_2024-06-12_18-30-00.mp4", 0.7)
add("music", "Telegram Desktop/track.mp3", 3, genres=["Pop"])
add("audiobook", "Telegram Desktop/Аудиокнига - Пикник на обочине.mp3", 420)
add("video", "Видео/Мультфильмы/Ну, погоди! (1969)/Выпуск 01.avi", 10)
add("video", "Видео/Мультфильмы/Ну, погоди! (1969)/Выпуск 02.avi", 10)
add("own_video", "Видео/VID_20230101_000100.mp4", 1)
add("own_video", "Видео/IMG_5501.MOV", 1, {"com.apple.quicktime.make": "Apple"})
add("video", "Видео/Интерстеллар (2014).mkv", 169)
add("music", "Media/Mix/Song A.mp3", 3.2, genres=["Rock"])
add("music", "Media/Mix/Song B.mp3", 4.1)
add("audiobook", "Media/Mix/Audiobook - The Martian.m4b", 650, chapters=20)
add("video", "Media/Mix/The.Martian.2015.1080p.mkv", 144)
add("own_video", "Media/Mix/PXL_20240101_120000000.mp4", 1)
add("video", "Kids/Bluey/Season 1/Bluey S01E01 Magic Xylophone.mp4", 7)
add("video", "Kids/Bluey/Season 1/Bluey S01E02 Hospital.mp4", 7)
add("own_video", "Kids/Утренник/VID_20231225_101500.mp4", 15)
add("own_video", "Kids/Утренник/VID_20231225_103000.mp4", 12)
add("video", "Concerts/Queen - Live at Wembley (1986).mkv", 120)
add("music", "Concerts/Queen - Live at Wembley (1986) - Audio/01 - One Vision.flac", 5, genres=["Rock"])

with open(sys.argv[1], "w", encoding="utf-8") as f:
    for r in rows:
        f.write(json.dumps(r, ensure_ascii=False) + "\n")
from collections import Counter
print(len(rows), Counter(r["kind"] for r in rows), file=sys.stderr)
