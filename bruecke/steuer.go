// Der Steuersocket: die Leitung zu drahtpost.
//
// Zeilenweise Text, in beide Richtungen. Das genuegt hier: es sind fuenf
// Befehle und vier Ereignisse, und ein Protokoll, das man mit nc lesen und
// schreiben kann, laesst sich auf dem Geraet selbst pruefen -- ohne
// Werkzeug, ohne zweites Telegram-Konto.
//
//	echo klingeln Anna | nc -U ~/.pytelegram/bruecke.sock
package main

import (
	"bufio"
	"fmt"
	"net"
	"strings"
	"sync"
)

type steuerung struct {
	mu    sync.Mutex
	hoerer []net.Conn
}

func (s *steuerung) anmelden(c net.Conn) {
	s.mu.Lock()
	s.hoerer = append(s.hoerer, c)
	s.mu.Unlock()
}

func (s *steuerung) abmelden(c net.Conn) {
	s.mu.Lock()
	for i, h := range s.hoerer {
		if h == c {
			s.hoerer = append(s.hoerer[:i], s.hoerer[i+1:]...)
			break
		}
	}
	s.mu.Unlock()
}

// senden schickt eine Ereigniszeile an alle, die zuhoeren.
func (s *steuerung) senden(zeile string) {
	s.mu.Lock()
	hoerer := append([]net.Conn(nil), s.hoerer...)
	s.mu.Unlock()
	for _, h := range hoerer {
		_, _ = fmt.Fprintf(h, "ereignis %s\n", zeile)
	}
}

func steuerungStarten(pfad string, b *sipBruecke) (*steuerung, error) {
	l, err := lauschenUnix(pfad)
	if err != nil {
		return nil, err
	}
	s := &steuerung{}
	b.mu.Lock()
	b.melder = s.senden
	b.mu.Unlock()
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				melden("Steuersocket: Accept: %v", err)
				return
			}
			melden("drahtpost verbunden")
			s.anmelden(c)
			go func() {
				defer func() { s.abmelden(c); _ = c.Close() }()
				leser := bufio.NewScanner(c)
				for leser.Scan() {
					antwort := befehl(b, strings.TrimSpace(leser.Text()))
					if _, err := fmt.Fprintln(c, antwort); err != nil {
						return
					}
				}
			}()
		}
	}()
	return s, nil
}

func befehl(b *sipBruecke, zeile string) string {
	if zeile == "" {
		return "ok"
	}
	wort, rest, _ := strings.Cut(zeile, " ")
	rest = strings.TrimSpace(rest)
	melden("Befehl: %s", zeile)
	switch wort {
	case "klingeln":
		if err := b.klingeln(rest); err != nil {
			return "fehler " + err.Error()
		}
		return "ok"
	case "klingelt":
		b.klingelt()
		return "ok"
	case "annehmen":
		if err := b.annehmen(); err != nil {
			return "fehler " + err.Error()
		}
		return "ok"
	case "auflegen":
		if rest == "" {
			rest = "beendet"
		}
		b.auflegen(rest)
		return "ok"
	case "stand":
		b.mu.Lock()
		defer b.mu.Unlock()
		return fmt.Sprintf("stand registriert=%v laeuft=%v angenommen=%v ton=%v pakete=%d",
			b.registriert, b.laeuft, b.angenommen, b.ton.offen, b.pakete)
	}
	return "fehler unbekannter Befehl " + wort
}
