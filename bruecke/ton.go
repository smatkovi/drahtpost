// Ton: der Draht zum Tonprozess.
//
// Der Tonprozess (tgcalls) und das Telefon takten beide, aber nach
// verschiedenen Uhren. Die Uhr, nach der sich hier alles richtet, ist die
// des Telefons: seine RTP-Pakete kommen alle 20 ms, weil sein Kodierer an
// der Tonhardware haengt. Jedes Paket schiebt zwei 10-ms-Rahmen in den
// Socket; der Tonprozess haengt lesend daran und wird damit von der
// Telefonhardware getaktet, ohne eine einzige eigene Zeitschleife.
//
// Zurueck kommt im selben Schritt ein Rahmen -- der Tonprozess arbeitet im
// Gleichschritt: lesen, aufnehmen, abspielen, schreiben. Deshalb braucht
// es hier keinen Sendetakt: es geht genau so viel hinaus, wie hereinkam.
package main

import (
	"encoding/binary"
	"io"
	"net"
	"sync"
	"time"
)

const (
	tonRate   = 16000 // was tgcalls bekommt
	tonRahmen = 160   // 10 ms bei 16 kHz
)

// tonDraht haelt die Verbindung zum Tonprozess.
type tonDraht struct {
	mu   sync.Mutex
	c    net.Conn
	offen bool

	// Was der Tonprozess abgespielt haben will, bis ein RTP-Paket voll ist.
	hinaus []int16
}

func (t *tonDraht) setzen(c net.Conn) {
	t.mu.Lock()
	alt := t.c
	t.c = c
	t.offen = c != nil
	t.hinaus = t.hinaus[:0]
	t.mu.Unlock()
	if alt != nil {
		_ = alt.Close()
	}
}

// schreiben schiebt aufgenommene Samples (16 kHz) an den Tonprozess.
func (t *tonDraht) schreiben(s []int16) error {
	t.mu.Lock()
	c := t.c
	t.mu.Unlock()
	if c == nil {
		return nil
	}
	roh := make([]byte, len(s)*2)
	for i, w := range s {
		binary.LittleEndian.PutUint16(roh[i*2:], uint16(w))
	}
	_ = c.SetWriteDeadline(time.Now().Add(200 * time.Millisecond))
	_, err := c.Write(roh)
	return err
}

// lesen holt einen Rahmen, den der Tonprozess abgespielt haben will.
// Blockierend: der Tonprozess antwortet auf jeden geschriebenen Rahmen mit
// genau einem.
func (t *tonDraht) lesen(c net.Conn, ziel []int16) error {
	roh := make([]byte, len(ziel)*2)
	if _, err := io.ReadFull(c, roh); err != nil {
		return err
	}
	for i := range ziel {
		ziel[i] = int16(binary.LittleEndian.Uint16(roh[i*2:]))
	}
	return nil
}

// tonSocket nimmt die Verbindung des Tonprozesses entgegen.
//
// Immer nur eine: meldet sich ein neuer Tonprozess, loest er den alten ab.
// Nach einem Absturz waere sonst der tote Draht der, an dem alles haengt.
func (b *sipBruecke) tonSocket(pfad string) error {
	l, err := lauschenUnix(pfad)
	if err != nil {
		return err
	}
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				melden("Tonsocket: Accept: %v", err)
				return
			}
			melden("Tonprozess verbunden")
			b.ton.setzen(c)
			go b.tonLesen(c)
		}
	}()
	return nil
}

// tonLesen nimmt entgegen, was der Tonprozess abspielen will, und schickt
// es als RTP ans Telefon -- sobald 20 ms beisammen sind.
func (b *sipBruecke) tonLesen(c net.Conn) {
	rahmen := make([]int16, tonRahmen)
	for {
		if err := b.ton.lesen(c, rahmen); err != nil {
			melden("Tonprozess weg: %v", err)
			b.ton.mu.Lock()
			if b.ton.c == c {
				b.ton.c, b.ton.offen = nil, false
			}
			b.ton.mu.Unlock()
			_ = c.Close()
			return
		}
		b.ton.mu.Lock()
		b.ton.hinaus = append(b.ton.hinaus, rahmen...)
		// Ein RTP-Paket sind 20 ms: 320 Samples bei 16 kHz.
		voll := len(b.ton.hinaus) >= tonRahmen*2
		var paket []int16
		if voll {
			paket = make([]int16, tonRahmen*2)
			copy(paket, b.ton.hinaus[:tonRahmen*2])
			b.ton.hinaus = b.ton.hinaus[tonRahmen*2:]
		}
		b.ton.mu.Unlock()
		if voll {
			b.rtpSenden(paket)
		}
	}
}
