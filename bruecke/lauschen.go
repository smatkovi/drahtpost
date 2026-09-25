// Eingehende Verbindungen auf einem 2.6.32-Kernel.
//
// Das ist die einzige Stelle, an der Gos Mindestanforderung "Kernel >= 3.2"
// auf Harmattan wirklich beisst:
//
//     accept unix .../bruecke.sock: accept4: function not implemented
//
// accept4 kam in den ARM-Syscall-Tisch erst nach 2.6.32, und modernes Go
// hat den Rueckfall auf das alte accept entfernt, als es die
// Mindestanforderung anhob. syscall.Accept hilft nicht: es ruft ebenfalls
// nur accept4. Bleibt der rohe Aufruf -- accept ist auf ARM-EABI die
// Nummer 285.
//
// Dieselbe Loesung steht in harbour-whatsapp/backend/listen_meego.go, dort
// fuer TCP. Hier fuer Unix-Sockets, sonst Zeile fuer Zeile dasselbe.
//
// Der Preis ist ein OS-Thread, der im accept blockiert. Fuer zwei lokale
// Sockets mit je einer Verbindung ist das kein Thema.
package main

import (
	"fmt"
	"net"
	"os"
	"sync"
	"syscall"
	"unsafe"
)

const sysAcceptARM = 285

type meegoLauscher struct {
	fd   int
	pfad string
	addr net.Addr
	once sync.Once
	mu   sync.Mutex
	zu   bool
}

func (l *meegoLauscher) Accept() (net.Conn, error) {
	for {
		l.mu.Lock()
		zu := l.zu
		l.mu.Unlock()
		if zu {
			return nil, net.ErrClosed
		}
		var rsa syscall.RawSockaddrAny
		groesse := uint32(syscall.SizeofSockaddrAny)
		nfd, _, errno := syscall.Syscall(sysAcceptARM, uintptr(l.fd),
			uintptr(unsafe.Pointer(&rsa)), uintptr(unsafe.Pointer(&groesse)))
		if errno != 0 {
			if errno == syscall.EINTR {
				continue // Signal waehrend des Wartens -- einfach nochmal
			}
			return nil, fmt.Errorf("accept: %v", errno)
		}
		syscall.SetNonblock(int(nfd), true)
		syscall.CloseOnExec(int(nfd))
		f := os.NewFile(nfd, "verbindung")
		c, err := net.FileConn(f)
		f.Close() // FileConn dupliziert; das Original wird nicht mehr gebraucht
		if err != nil {
			return nil, err
		}
		return c, nil
	}
}

func (l *meegoLauscher) Close() error {
	var err error
	l.once.Do(func() {
		l.mu.Lock()
		l.zu = true
		l.mu.Unlock()
		err = syscall.Close(l.fd)
		_ = os.Remove(l.pfad)
	})
	return err
}

func (l *meegoLauscher) Addr() net.Addr { return l.addr }

// lauschenUnix oeffnet einen Unix-Socket ohne accept4.
func lauschenUnix(pfad string) (net.Listener, error) {
	_ = os.Remove(pfad)
	fd, err := syscall.Socket(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		return nil, err
	}
	syscall.CloseOnExec(fd)
	if err = syscall.Bind(fd, &syscall.SockaddrUnix{Name: pfad}); err != nil {
		syscall.Close(fd)
		return nil, err
	}
	// Nur der Benutzer selbst: ueber diesen Socket laesst sich telefonieren.
	if err = os.Chmod(pfad, 0o600); err != nil {
		syscall.Close(fd)
		return nil, err
	}
	if err = syscall.Listen(fd, 8); err != nil {
		syscall.Close(fd)
		return nil, err
	}
	return &meegoLauscher{fd: fd, pfad: pfad, addr: &net.UnixAddr{Name: pfad, Net: "unix"}}, nil
}
