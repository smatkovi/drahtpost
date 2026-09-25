// drahtpost-bruecke: gibt Telegram-Anrufe an die Telefon-App des N9/N950.
//
// Laeuft als eigener Prozess, von drahtpost bei Bedarf gestartet. Eigener
// Prozess, weil er zwei Dinge tut, die man einzeln neu starten koennen
// will: er haelt einen SIP-Server (den das Telefon dauerhaft anmeldet) und
// er reicht Ton durch (der nur waehrend eines Gespraechs fliesst). Stuerzt
// der Tonprozess ab, bleibt die Anmeldung stehen.
package main

import (
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"
)

func main() {
	verzeichnis := filepath.Join(os.Getenv("HOME"), ".pytelegram")
	if err := os.MkdirAll(verzeichnis, 0o755); err != nil {
		fmt.Println("Verzeichnis:", err)
		os.Exit(1)
	}
	b, err := brueckeStarten()
	if err != nil {
		fmt.Println("Bruecke:", err)
		os.Exit(1)
	}
	bruecke = b
	if err := b.tonSocket(filepath.Join(verzeichnis, "tonbruecke.sock")); err != nil {
		fmt.Println("Tonsocket:", err)
		os.Exit(1)
	}
	if _, err := steuerungStarten(filepath.Join(verzeichnis, "bruecke.sock"), b); err != nil {
		fmt.Println("Steuersocket:", err)
		os.Exit(1)
	}
	go b.stilleNachschieben()

	// Beim Beenden das Telefon nicht klingelnd zuruecklassen.
	zeichen := make(chan os.Signal, 1)
	signal.Notify(zeichen, syscall.SIGINT, syscall.SIGTERM)
	<-zeichen
	b.auflegen("beendet")
	time.Sleep(100 * time.Millisecond)
}

// stilleNachschieben haelt den Takt aufrecht, wenn vom Telefon nichts
// kommt.
//
// Der Tonprozess liest blockierend und wird dadurch getaktet. Bleibt das
// RTP aus -- Telefon noch nicht am Ton, Paket verloren, Geraet kurz
// ueberlastet --, stuende er still, und tgcalls schickte in dieser Zeit
// gar nichts. Lieber Stille als Stillstand.
func (b *sipBruecke) stilleNachschieben() {
	stille := make([]int16, tonRahmen*2)
	takt := time.NewTicker(20 * time.Millisecond)
	defer takt.Stop()
	for range takt.C {
		b.mu.Lock()
		luecke := b.laeuft && time.Since(b.letztesRTP) > 60*time.Millisecond
		if luecke {
			b.letztesRTP = time.Now()
		}
		b.mu.Unlock()
		if luecke {
			_ = b.ton.schreiben(stille)
		}
	}
}
