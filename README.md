# Drahtpost

Ein Telegram-Daemon für Nokia N9 und N950 (MeeGo 1.2 Harmattan), in Rust.

Er ersetzt `telegram_daemon.py` aus dem PyTeleGram-Paket, ohne dass
irgendetwas darum herum geändert werden muss: derselbe Unix-Socket,
dieselben zwanzig Befehle, dieselben Feldnamen. Die Qt-Oberfläche von
PyTeleGram und die [Nachrichtenbrücke](https://github.com/smatkovi/nachrichtenbruecke)
merken den Unterschied nicht.

## Warum

Auf einem Gerät mit 1 GB, von dem nach dem Hochfahren gut 230 MB frei
sind, ist ein Telethon-Daemon je Protokoll nicht zu bezahlen — der
Python-Prozess belegt rund 55 MB. [grammers](https://github.com/Lonami/grammers)
spricht MTProto in reinem Rust: kein TDLib, kein C++, kein Python.

## Die Anmeldung bleibt

Der Auth-Key einer Telegram-Sitzung hängt am Rechenzentrum, nicht an der
Bibliothek, die ihn ausgehandelt hat. Drahtpost liest beim ersten Start
`~/.pytelegram/session.session` (Telethons SQLite-Datei), nimmt die 256
Bytes daraus und legt sie in eine grammers-Sitzung. Auf dem Gerät
nachgemessen: angemeldet, 861 Dialoge, ohne eine einzige SMS.

Die Telethon-Datei wird dabei nur gelesen. Sie bleibt liegen, und mit ihr
der Weg zurück.

## Bauen

    tools/build.sh

Baut auf einem Fremdrechner gegen `armv7-unknown-linux-musleabi`,
vollständig statisch. Warum musl und nicht glibc, und warum `rust-lld`:
siehe `tools/cross.env`.

## Stummgeschaltete Chats

Telegram führt die Stummschaltung am Dialog: in `notify_settings` steht
ein `mute_until`, für „auf immer“ 2147483647. Drahtpost merkt sich diesen
Zeitpunkt je Chat in `~/.pytelegram/stumm.json` — gemerkt wird der
Zeitpunkt und kein Ja/Nein, damit „acht Stunden stumm“ von selbst
abläuft. Gefüllt wird die Tabelle aus einem vollständigen Dialogdurchlauf
und danach laufend aus `updateNotifySettings`.

`new_message` und `message_edited` tragen deshalb zwei Felder, die der
Python-Daemon nicht hatte: `muted` und `mentioned`. Die
[Nachrichtenbrücke](https://github.com/smatkovi/nachrichtenbruecke) hält
damit stumme Chats aus der Nachrichten-App; Erwähnungen und Antworten an
einen selbst kommen weiter durch. Die Oberfläche von PyTeleGram liest die
Felder nicht und bekommt unverändert alles.

## Was fehlt

Sprach- und Videoanrufe. Telegram trennt das sauber — TDLib beziehungs-
weise die rohe API trägt die Signalisierung, die Medien laufen über
`tgcalls` auf WebRTC-Basis. Das ist ein eigenes Vorhaben.
