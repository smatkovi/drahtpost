# Sprachanrufe: was gemessen ist, und was noch fehlt

Telegram-Anrufe laufen nicht über ein Standardprotokoll, sondern über
MTProto: Signalisierung mit `phone.requestCall` und Diffie-Hellman, danach
Opus-Rahmen über Telegrams eigene Reflektoren. Bevor davon irgendetwas
gebaut wird, waren zwei Fragen zu klären — die eine ist beantwortet.

## Beantwortet: der Ton ist bezahlbar

Anders als bei WhatsApp (dessen MLow-Codec 88 % eines Kerns braucht, nach
fünf Optimierungsrunden) ist Opus auf diesem Gerät kein Engpass. Gemessen
auf der N950 mit `opus-messen.c`, ein Rahmen sind 20 ms:

| Bau | kodieren | dekodieren | zusammen | vom Budget |
|---|---|---|---|---|
| Festkomma, 48 kHz, Komplexität 10 | 3,68 ms | 0,37 ms | 4,06 ms | 20 % |
| **Festkomma, 48 kHz, Komplexität 5** | 1,83 ms | 0,37 ms | **2,20 ms** | **11 %** |
| Festkomma, 16 kHz, Komplexität 5 | 1,37 ms | 0,12 ms | 1,49 ms | 7 % |
| Gleitkomma, 48 kHz, Komplexität 5 | 4,19 ms | 0,86 ms | 5,05 ms | 25 % |

Drei Schlüsse:

* **Festkomma, nicht Gleitkomma.** Auf diesem Kern ohne schnelle
  Fließkommaeinheit ist das ein Faktor 2,3.
* **NEON zahlt sich hier aus** — anders als beim MLow-Kodierer, wo
  handgeschriebene NEON-Kerne langsamer waren als Gos skalarer Code. Der
  Unterschied: libopus rechnet lange Ströme am Stück ab (Tonhöhensuche,
  NSQ), nicht 80er-Skalarprodukte mit Rückgabewert.
* In Hardware gibt es Opus nicht. Die DSP des OMAP3630 trägt nur Video-
  und Bildknoten (`/lib/dsp`: H.264, MPEG-4, WMV9, JPEG); die
  hardwarenahen Audiocodecs des Geräts sind AMR und G.729 (~600× schneller
  als Echtzeit), die aber niemand hier sprechen will.

`opus-bauen.sh` übersetzt libopus ohne autotools: die Quelllisten stehen
in den `*.mk` des Projekts, übersetzt wird direkt mit clang gegen das
Harmattan-Sysroot. So hat man die Fahnen wirklich in der Hand.

## Offen: nimmt der Server das alte Protokoll noch an?

Das ist die Frage, an der alles hängt. Es gibt zwei Generationen von
Telegrams Sprachanrufen:

* **libtgvoip** (bis etwa 2020): C++11, Opus über einen eigenen
  UDP-Transport, ohne WebRTC — kreuzübersetzbar, überschaubar.
* **tgcalls** (heute): auf WebRTC aufgebaut, auf diesem Gerät aussichtslos.

Ob Telegrams Server für Einzelgespräche noch eine alte Protokollversion
annehmen, lässt sich nicht aus der Ferne beantworten, sondern nur
ausprobieren: ein `phone.requestCall` mit niedriger Protokollstufe
absetzen und sehen, ob der Server es annimmt und beim Angerufenen etwas
klingelt. Ein Tagwerk — und es spart im Zweifel Wochen.

## Der Weg danach

1. Signalisierung in `drahtpost` (grammers kennt die `phone.*`-Aufrufe aus
   dem Schema; Diffie-Hellman und die Emoji-Prüfsumme kommen dazu).
2. Medien: libtgvoip kreuzübersetzen und über seine
   „custom audio IO"-Schnittstelle anbinden — oder das Paketformat in Rust
   nachbauen, wenn sich zeigt, dass es überschaubar bleibt.
3. Den Ton an die SIP-Brücke der [Nachrichtenbrücke](https://github.com/smatkovi/nachrichtenbruecke)
   geben, die schon WhatsApp-Anrufe in die Anrufansicht des Geräts trägt.
   Sie ist protokollblind: sie will PCM, sonst nichts.
