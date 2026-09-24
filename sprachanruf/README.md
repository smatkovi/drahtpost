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

## libtgvoip: übersetzt vollständig — eine Wand steht noch

`libtgvoip-bauen.sh` übersetzt die Bibliothek für Harmattan. Stand:
**27 von 27 Dateien, kein Fehler.** Das ist mehr, als man erwarten durfte.

Der Weg dorthin, mit den drei Hindernissen und ihren Lösungen:

* **`TGVOIP_NO_DSP`** wirft den mitgelieferten WebRTC-Ton-Teil hinaus
  (276 Dateien, 5 MB): Echo, Rauschen und Aussteuerung macht der
  Sprachpfad des Geräts, sobald der Ton über die SIP-Brücke läuft.
* **`pthread_setname_np`** gibt es erst ab glibc 2.12; Harmattan hat 2.10.
  `tgvoip-kompat.h` macht daraus einen Leerlauf — die Funktion benennt nur
  Fäden für den Debugger.
* **`PRIx64`** braucht in dieser Zeit noch `-D__STDC_FORMAT_MACROS`, und
  clang ist bei verengenden Initialisierungen strenger als GCC
  (`-Wno-c++11-narrowing`).

Dazu gibt es mit `TGVOIP_USE_CALLBACK_AUDIO_IO` genau die Schnittstelle,
die wir brauchen: `SetAudioDataCallbacks(eingang, ausgang)` — PCM rein,
PCM raus, der Rest ist Sache der Brücke.

### Was fehlt: eine C++11-Bibliothek mit weichem Gleitkomma-ABI

Die Bibliothek übersetzt, aber sie bindet nicht. Der Grund ist eine Zange:

* Das **Sysroot** von 2009 hat GCC 4.4 und damit keine C++11-Bibliothek —
  es fehlen `std::__cxx11::basic_string`, `__throw_bad_function_call`,
  `_List_node_base::_M_hook` und andere.
* Die **GCC-14-Kreuzkette** bringt sie mit, ist aber `--with-float=hard`
  gebaut. Damit liegen ihre Objekte im harten ABI, unsere
  clang-Objekte (und Gos) im weichen: *"uses VFP register arguments, … does
  not"*.
* Alles auf hart umzustellen scheidet aus: dann riefe numerischer Code die
  **libm des Geräts** mit der falschen Aufrufkonvention — Gleitkommazahlen
  in VFP-Registern statt in Kernregistern. Das stürzt nicht ab, es rechnet
  falsch.

Es braucht also eine C++11-Laufzeit für `armv7 softfp`. Zwei Wege:

1. **libc++ für das Ziel bauen** (clang + cmake, llvm-project als Quelle).
   Bounded, wiederverwendbar — und der Schlüssel für *jede* moderne
   C++-Portierung auf dieses Gerät, tgcalls eingeschlossen.
2. Eine GCC-Kreuzkette mit `--with-float=softfp` bauen. Länger, aber
   vertrauter.

Vorher lohnt keine Arbeit an der Signalisierung: ohne Laufzeit gibt es
kein Binär.
