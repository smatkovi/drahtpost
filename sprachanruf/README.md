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

### Es läuft: libtgvoip 2.5 auf der N950

```
libtgvoip 2.5
Verbindungsstufe (min/max): 92/92
VoIPController angelegt
Ton-Rueckrufe gesetzt
wieder abgeraeumt -- libtgvoip laeuft auf diesem Geraet
```

Damit ist die Machbarkeit der Tonseite vollständig belegt: die Bibliothek
übersetzt, bindet und läuft, die Rückruf-Tonschnittstelle steht, und Opus
kostet 11 % des Budgets. Was bleibt, ist Signalisierung — und die eine
offene Frage unten.

### Der Umweg, der keiner war: das Gleitkomma-ABI

Zwischendurch sah es aus, als bräuchte es eine C++11-Laufzeit mit
**weichem** Gleitkomma-ABI, weil die Bindung an den Symbolen der alten
libstdc++ scheiterte und die GCC-14-Kette hart gebaut ist. Das war ein
Trugschluss, und er ist es wert, hier zu stehen:

```
readelf -A auf dem Sysroot:
libc.so.6       Tag_ABI_VFP_args: VFP registers
libm.so.6       Tag_ABI_VFP_args: VFP registers
libQtCore.so.4  Tag_ABI_VFP_args: VFP registers
libstdc++.so.6  Tag_ABI_VFP_args: VFP registers
```

**Harmattan ist hart gleitkommig.** Der Sonderfall ist Go, dessen
ARM-Konvention Gleitkommazahlen in Kernregistern reicht — deshalb braucht
*cgo* clang mit `-mfloat-abi=softfp`. Für C und C++ gilt die harte Kette,
und dann bindet `-static-libstdc++` die fehlende C++11-Bibliothek einfach
mit ein, genau wie bei der Qt-Oberfläche dieses Projekts.

Zwei Fallen dabei, beide bezahlt:

* Wer Übersetzerausgabe durch `head` leitet, killt den Übersetzer per
  SIGPIPE — bei zwei warnungsreichen Opus-Dateien fehlte danach
  stillschweigend das Objekt, und der Binder meldete fehlende Symbole,
  die es gar nicht sein konnten.
* `Stop()` vor `delete` beim VoIPController, sonst bricht er ab. Die
  Bibliothek sagt es selbst, in Großbuchstaben.

### Nicht mehr nötig: eine Laufzeit mit weichem ABI

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

Beide Wege — libc++ für das Ziel bauen oder eine GCC-Kette mit
`--with-float=softfp` — haben sich damit erledigt. Gebraucht wird keiner
von beiden.

## libwebrtc baut für Harmattan

`webrtc/bauen.sh` übersetzt libwebrtc für die N950. Ergebnis:

```
libwebrtc.a   43 MB, 2845 Objekte
Tag_FP_arch: VFPv3   Tag_ABI_VFP_args: VFP registers
```

Also genau das ABI, das Harmattan spricht. Zwei Dinge gaben den Ausschlag,
beide hatte ich vorher unterschätzt: WebRTC bringt **seine eigene libc++**
mit (`use_custom_libcxx`) — das Laufzeitproblem, an dem libtgvoip fast
gescheitert wäre, löst es selbst — und es kann von Haus aus armv7
hard-float mit NEON.

### Die elf Lücken von 2009

Alle gelöst durch eine Kompatibilitätsschicht, die über
`build/config/compiler/BUILD.gn` in jede Übersetzung gezogen wird:

| Was fehlte | seit wann es das gibt | wie gelöst |
|---|---|---|
| `CLOCK_BOOTTIME`, `CLOCK_MONOTONIC_RAW` | Kernel 2.6.39 / 2.6.28 | Zahlen nachdefiniert; fehlt die Uhr, fällt der Code selbst zurück |
| `pthread_setname_np`, `getname_np` | glibc 2.12 | Leerlauf-Attrappe in `schicht/pthread.h`, C **und** C++ |
| `PRIX32` & Co. | — | `__STDC_FORMAT_MACROS` vor dem ersten Einbinden |
| BoringSSLs NEON-Erkennung | braucht `sys/auxv.h` | `OPENSSL_STATIC_ARMCAP` — der OMAP3630 *hat* NEON |
| `getrandom` | Kernel 3.17 | Syscall-Nummer; scheitert mit ENOSYS, BoringSSL nimmt `/dev/urandom` |
| `sys/auxv.h` | glibc 2.16 | selbst geschrieben: liest `/proc/self/auxv` |
| `TCP_USER_TIMEOUT` | Kernel 2.6.37 | Zahl; `setsockopt` lehnt zur Laufzeit ab, das verkraftet der Aufrufer |
| `atan2l`, `logl`, … | — | auf ARM setzt glibc `__NO_LONG_DOUBLE_MATH`, und `math.h` erklärt die `…l`-Funktionen **gar nicht**. Da `long double` dort `double` ist, sind die Weiterleitungen in `schicht/math.h` exakt |
| `static_assert` in C | C11 | auf `_Static_assert` |
| `v4l2_capability.device_caps` | Kernel 3.3 | Kopfdateien des Baurechners (die Kamera öffnen wir nie) |
| X11-Bildschirmaufnahme | — | `rtc_use_x11 = false` |

### Zwei Umwege, die Zeit gekostet haben

* **Die Schicht muss auf ARM beschränkt bleiben.** Gilt sie auch für den
  Baurechner, zieht sich bindgen (Rust) C++-Kopfdateien in den Parser und
  scheitert an libc++-Vorlagen. `enable_rust = false` ist übrigens keine
  Lösung: WebRTC benutzt Rust in eigenen Zielen (`api/units`).
* **Die `long double`-Mathematik gehört in ein eigenes `math.h` mit
  `#include_next`**, nicht in den Zwangs-Include — aus demselben Grund.
  Und sie muss `math.h` **zuerst** einbinden: `__NO_LONG_DOUBLE_MATH`
  entsteht ja erst dadurch.

## tgcalls: gegen welches WebRTC?

Das fertige `libwebrtc.a` ist Upstream vom 24.09.2026. Darauf lässt sich
tgcalls **nicht** übersetzen, und zwar aus einem Grund, der nichts mit
Harmattan zu tun hat: WebRTC hat inzwischen `rtc::` und `cricket::` in
`webrtc::` zusammengelegt und sigslot gelöscht. tgcalls baut seit jeher
gegen **tg_owt**, Telegrams eigenen WebRTC-Abzug, und der steht auf einem
älteren Stand.

Bevor ich mich für einen Weg entschieden habe, habe ich die Entfernung
gemessen: eine Kopie von tgcalls maschinell umbenannt (`rtc::` →
`webrtc::`, `cricket::` → `webrtc::`), sigslot aus tg_owt beigelegt und
übersetzt.

```
ohne Umbenennung:  6 von 41 Dateien
mit Umbenennung:  12 von 41 Dateien, 110 verschiedene Fehler
```

Die 110 sind nicht die Reste einer Umbenennung. Es sind drei Jahre
Schnittstellenwandel:

* `VoiceChannel`/`VideoChannel` gibt es nicht mehr (ChannelManager ist weg)
* die sigslot-Signale der Transporte (`SignalWritableState`,
  `SignalSentPacket`, `SignalRouteChange`) sind durch Rückrufe ersetzt
* `ContentInfo`, `Codec`, `JsepIceCandidate` sind umgebaut
* `PacketOptions`, `CryptoOptions`, `SentPacket`, `AsyncResolverInterface`
  umbenannt oder verschoben

Das wäre ein Port von tgcalls auf heutiges WebRTC — Telegrams Arbeit, nicht
unsere. **Also tg_owt.** Und das ist gar nicht schlimm:

```
Upstream-WebRTC (depot_tools):   13 GB
tg_owt mit allen Submodulen:    120 MB
```

tg_owt ist ein gewöhnliches CMake-Projekt und erkennt armv7+NEON selbst.
Im nicht-gepackten Bau braucht es von OpenSSL, Opus, FFmpeg und libjpeg
nur die **Kopfdateien**; gebunden wird erst beim Programm. Die
Kompatibilitätsschicht der elf Lücken geht per `CMAKE_CXX_FLAGS` hinein,
die libc++ und das clang bleiben die aus dem Upstream-Baum — der Baum
bleibt also stehen, auch wenn `libwebrtc.a` selbst nicht mehr gebraucht
wird.
