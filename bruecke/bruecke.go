// SIP-Bruecke fuer Telegram-Anrufe auf Harmattan.
//
// Der Gedanke ist von harbour-whatsapp uebernommen, wo er sich bewaehrt
// hat: Harmattan bringt telepathy-sofiasip mit, also kann das N9/N950
// SIP-Teilnehmer sein -- und ein SIP-Anruf laeuft durch die *systemeigene*
// Anrufansicht. Klingeln am Sperrbildschirm, Annehmen ohne die App zu
// oeffnen, Naeherungssensor, Lautstaerketasten -- und vor allem die
// Hoermuschel statt des Lautsprechers. Das alles selbst zu bauen waere
// aussichtslos; es zu benutzen kostet einen kleinen SIP-Server.
//
// Genau die beiden Beschwerden nach dem ersten echten Telegram-Anruf
// ("es ist am Lautsprecher", "ich sehe die Call-UI nicht") loest dieser
// Weg auf einen Schlag, weil beides gar nicht mehr unsere Sache ist.
//
//	Telefon (telepathy-sofiasip)  <--SIP/RTP-->  diese Bruecke  <-->  tonprozess
//	    registriert sich bei 127.0.0.1:5062                     PCM ueber Unix-Socket
//
// Port 5062, nicht 5060: dort sitzt schon die WhatsApp-Bruecke. Zwei
// sofiasip-Konten mit verschiedenen Proxy-Ports koennen nebeneinander
// bestehen, ein Port nicht zweimal.
//
// Bewusst nur auf 127.0.0.1: die Bruecke nimmt jede Registrierung an, ohne
// Passwort. Auf der Loopback-Schnittstelle ist das vertretbar -- wer dort
// Pakete schicken kann, laeuft ohnehin schon als Benutzer auf dem Geraet.
package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"net"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/emiago/sipgo"
	"github.com/emiago/sipgo/sip"
	"github.com/pion/rtp"
)

const (
	sipHost      = "127.0.0.1"
	sipPort      = 5062
	rtpPort      = 40102
	sipRate      = 8000 // G.711 ist immer 8 kHz
	rtpFrame     = 160  // 20 ms bei 8 kHz
	nutzlastPCMU = 0
	// Unser Benutzerteil im Kontakt -- muss zum Konto passen, das
	// telegram-einrichten anlegt.
	sipNutzer = "telegram"
	sipWirt   = "telegram.local"
)

// ---------------------------------------------------------------- G.711 µ-law

func muLawKodieren(wert int16) byte {
	const bias = 0x84
	const clip = 32635
	vorzeichen := byte(0)
	if wert < 0 {
		wert = -wert
		vorzeichen = 0x80
	}
	if wert > clip {
		wert = clip
	}
	wert += bias
	exponent := byte(7)
	for maske := int16(0x4000); wert&maske == 0 && exponent > 0; maske >>= 1 {
		exponent--
	}
	mantisse := byte((wert >> (exponent + 3)) & 0x0F)
	return ^(vorzeichen | (exponent << 4) | mantisse)
}

var muLawTabelle [256]int16

func init() {
	for i := 0; i < 256; i++ {
		u := ^byte(i)
		vorzeichen := u & 0x80
		exponent := (u >> 4) & 0x07
		mantisse := u & 0x0F
		wert := (int16(mantisse) << 3) + 0x84
		wert <<= exponent
		wert -= 0x84
		if vorzeichen != 0 {
			wert = -wert
		}
		muLawTabelle[i] = wert
	}
}

func muLawDekodieren(b byte) int16 { return muLawTabelle[b] }

// ---------------------------------------------------------------- Umrechnung

// hoch verdoppelt die Abtastrate (8 -> 16 kHz) durch lineare Interpolation.
func hoch(in []int16) []int16 {
	aus := make([]int16, len(in)*2)
	for i := range in {
		aus[i*2] = in[i]
		if i+1 < len(in) {
			aus[i*2+1] = int16((int32(in[i]) + int32(in[i+1])) / 2)
		} else {
			aus[i*2+1] = in[i]
		}
	}
	return aus
}

// runter halbiert die Abtastrate (16 -> 8 kHz). Der Mittelwert zweier
// Nachbarn ist ein einfacher Tiefpass -- ohne den klingt das Ergebnis
// blechern, weil alles oberhalb von 4 kHz zurueckfaltet.
func runter(in []int16) []int16 {
	aus := make([]int16, len(in)/2)
	for i := range aus {
		aus[i] = int16((int32(in[i*2]) + int32(in[i*2+1])) / 2)
	}
	return aus
}

// ---------------------------------------------------------------- Die Bruecke

type sipBruecke struct {
	mu sync.Mutex

	ua  *sipgo.UserAgent
	srv *sipgo.Server
	cl  *sipgo.Client
	dc  *sipgo.DialogClientCache
	ds  *sipgo.DialogServerCache

	// Ein vom Telegram-Anruf ausgeloestes Klingeln (wir rufen das Telefon).
	sitzung *sipgo.DialogClientSession
	// Bricht ein noch klingelndes INVITE ab (CANCEL statt BYE).
	abbruch context.CancelFunc
	// Ein vom Telefon gewaehlter Anruf (das Telefon ruft uns).
	serverSitzung *sipgo.DialogServerSession

	angenommen bool
	laeuft     bool

	// Wo das Telefon erreichbar ist, aus seinem REGISTER.
	kontakt     *sip.Uri
	registriert bool

	rtp       *net.UDPConn
	gegen     *net.UDPAddr
	sequenz   uint16
	zeitmarke uint32
	ssrc      uint32

	ton tonDraht

	// Zaehler, nur zur Fehlersuche.
	pakete    uint64
	hinaus    uint64
	spitze    int16
	gemeldet  time.Time
	letztesRTP time.Time

	// Wohin Ereignisse gehen (Steuersocket).
	melder func(string)
}

var bruecke *sipBruecke

func melden(form string, args ...interface{}) {
	fmt.Printf("%s 📞 %s\n", time.Now().Format("15:04:05.000"), fmt.Sprintf(form, args...))
}

// ereignis schickt eine Zeile an drahtpost.
func (b *sipBruecke) ereignis(zeile string) {
	b.mu.Lock()
	m := b.melder
	b.mu.Unlock()
	melden("Ereignis: %s", zeile)
	if m != nil {
		m(zeile)
	}
}

func brueckeStarten() (*sipBruecke, error) {
	b := &sipBruecke{ssrc: uint32(time.Now().UnixNano())}
	ua, err := sipgo.NewUA(sipgo.WithUserAgent("drahtpost"))
	if err != nil {
		return nil, fmt.Errorf("UA: %w", err)
	}
	b.ua = ua
	if b.srv, err = sipgo.NewServer(ua); err != nil {
		return nil, fmt.Errorf("Server: %w", err)
	}
	if b.cl, err = sipgo.NewClient(ua); err != nil {
		return nil, fmt.Errorf("Client: %w", err)
	}
	// Die Dialogschicht von sipgo kuemmert sich um Tags, CSeq, ACK und BYE.
	// Von Hand ist das genau die Sorte Kleinkram, die man erst bemerkt,
	// wenn ein Geraet eigensinnig antwortet.
	kontakt := sip.ContactHeader{Address: sip.Uri{User: sipNutzer, Host: sipHost, Port: sipPort}}
	b.dc = sipgo.NewDialogClientCache(b.cl, kontakt)
	b.ds = sipgo.NewDialogServerCache(b.cl, kontakt)

	b.srv.OnRegister(b.beiRegister)
	b.srv.OnInvite(b.beiInvite)
	b.srv.OnBye(b.beiBye)
	b.srv.OnCancel(b.beiBye)
	b.srv.OnAck(func(req *sip.Request, tx sip.ServerTransaction) {
		b.mu.Lock()
		ss := b.serverSitzung
		b.mu.Unlock()
		if ss != nil {
			_ = ss.ReadAck(req, tx)
		}
	})
	// sofiasip schickt OPTIONS als Lebenszeichen an den Registrar. Bleibt
	// das unbeantwortet, haelt es die Bruecke irgendwann fuer tot und wirft
	// die Registrierung weg -- dann klingelt nichts mehr, ohne dass man
	// merkt warum.
	b.srv.OnOptions(func(req *sip.Request, tx sip.ServerTransaction) {
		_ = tx.Respond(sip.NewResponseFromRequest(req, 200, "OK", nil))
	})

	adr := net.JoinHostPort(sipHost, strconv.Itoa(sipPort))
	go func() {
		if lerr := b.srv.ListenAndServe(context.Background(), "udp", adr); lerr != nil {
			melden("SIP lauschen: %v", lerr)
		}
	}()
	if err = b.rtpOeffnen(); err != nil {
		return nil, fmt.Errorf("RTP: %w", err)
	}
	melden("SIP-Bruecke laeuft auf %s (RTP %d)", adr, rtpPort)
	return b, nil
}

func (b *sipBruecke) rtpOeffnen() error {
	c, err := net.ListenUDP("udp", &net.UDPAddr{IP: net.ParseIP(sipHost), Port: rtpPort})
	if err != nil {
		return err
	}
	b.rtp = c
	go b.rtpLesen()
	return nil
}

// kontaktZiel nimmt die Adresse, unter der sich das Telefon meldet, und
// ersetzt Wirt und Port durch die Quelle der Anmeldung.
//
// Das Telefon traegt in den Kontakt die Adresse einer seiner
// Schnittstellen ein, und welche das ist, entscheidet es selbst. Am
// Mobilfunk steht dort die Adresse aus dem Carrier-NAT, und unser INVITE
// ginge ueber den Router hinaus ins Netz statt ueber Loopback zum Telefon:
// es klingelte einfach nicht. Angemeldet hat es sich aber ueber 127.0.0.1.
// Jeder Registrar macht das; es heisst dort "received" und "rport".
func kontaktZiel(kontakt sip.Uri, quelle string) (sip.Uri, bool) {
	wirt, port, err := net.SplitHostPort(quelle)
	if err != nil || wirt == "" {
		return kontakt, false
	}
	p, err := strconv.Atoi(port)
	if err != nil || p <= 0 {
		return kontakt, false
	}
	if kontakt.Host == wirt && kontakt.Port == p {
		return kontakt, false
	}
	kontakt.Host, kontakt.Port = wirt, p
	return kontakt, true
}

func (b *sipBruecke) beiRegister(req *sip.Request, tx sip.ServerTransaction) {
	if k := req.Contact(); k != nil {
		uri := k.Address
		genannt := uri.String()
		uri, geaendert := kontaktZiel(uri, req.Source())
		b.mu.Lock()
		erste := !b.registriert
		b.kontakt = &uri
		b.registriert = true
		b.mu.Unlock()
		if geaendert {
			melden("Telefon nennt %s, meldet sich aber von %s -- wir klingeln dort", genannt, uri.String())
		} else {
			melden("Telefon registriert als %s", uri.String())
		}
		if erste {
			b.ereignis("registriert")
		}
	}
	antwort := sip.NewResponseFromRequest(req, 200, "OK", nil)
	// Lange genug, dass das Telefon nicht staendig neu anklopft, kurz
	// genug, dass ein Neustart auffaellt.
	antwort.AppendHeader(sip.NewHeader("Expires", "600"))
	_ = tx.Respond(antwort)
}

// beiInvite nimmt einen Anruf entgegen, den das Telefon gewaehlt hat.
//
// Das Konto des Telefons zeigt mit proxy-host=127.0.0.1 auf uns: jeder
// Anruf ueber dieses Konto landet als INVITE hier. Damit bekommen
// ausgehende Anrufe dieselbe systemeigene Anrufansicht wie eingehende --
// und der Weg ueber PulseAudio entfaellt fuer sie ganz.
func (b *sipBruecke) beiInvite(req *sip.Request, tx sip.ServerTransaction) {
	gewaehlt := req.Recipient.User
	melden("Telefon waehlt %q", gewaehlt)
	if gewaehlt == "" {
		_ = tx.Respond(sip.NewResponseFromRequest(req, 404, "Not Found", nil))
		return
	}
	b.mu.Lock()
	besetzt := b.laeuft || b.serverSitzung != nil || b.sitzung != nil
	b.mu.Unlock()
	if besetzt {
		_ = tx.Respond(sip.NewResponseFromRequest(req, 486, "Busy Here", nil))
		return
	}
	sitzung, err := b.ds.ReadInvite(req, tx)
	if err != nil {
		melden("INVITE nicht lesbar: %v", err)
		_ = tx.Respond(sip.NewResponseFromRequest(req, 400, "Bad Request", nil))
		return
	}
	b.gegenstelleAusSDP(string(req.Body()))
	b.mu.Lock()
	b.serverSitzung = sitzung
	b.angenommen = false
	b.zaehlerZuruecksetzen()
	b.mu.Unlock()
	_ = sitzung.Respond(sip.StatusTrying, "Trying", nil)
	// Ab hier fuehrt drahtpost: es loest den Telegram-Anruf aus und meldet
	// mit "klingelt"/"angenommen"/"beendet" zurueck.
	b.ereignis("waehlt " + gewaehlt)
}

func (b *sipBruecke) beiBye(req *sip.Request, tx sip.ServerTransaction) {
	b.mu.Lock()
	ss := b.serverSitzung
	b.mu.Unlock()
	if ss != nil {
		// Ein vom Telefon gewaehltes Gespraech: die Dialogschicht
		// beantwortet das BYE selbst.
		_ = ss.ReadBye(req, tx)
	} else {
		_ = tx.Respond(sip.NewResponseFromRequest(req, 200, "OK", nil))
	}
	b.mu.Lock()
	b.laeuft = false
	b.angenommen = false
	b.serverSitzung = nil
	b.mu.Unlock()
	melden("Telefon hat aufgelegt")
	b.ereignis("aufgelegt")
}

// klingeln schickt ein INVITE ans Telefon. Ab hier uebernimmt Harmattans
// eigene Anrufansicht -- samt Sperrbildschirm.
func (b *sipBruecke) klingeln(name string) error {
	b.mu.Lock()
	kontakt := b.kontakt
	b.zaehlerZuruecksetzen()
	b.mu.Unlock()
	if kontakt == nil {
		return fmt.Errorf("kein Telefon registriert")
	}
	// Der Anzeigename landet in der Anrufansicht. Er muss durch einen
	// SIP-URI passen, also bleiben nur unverfaengliche Zeichen stehen.
	// Der eigene From-Kopf ist noetig, damit in der Anrufansicht der Name
	// des Anrufers steht und nicht "telegram". Er muss aber ein tag
	// tragen: das tag ist die halbe Dialogkennung, und ohne es scheiterte
	// die Annahme mit "missing tag param in From header" -- das Telefon
	// klingelte, man hob ab, und im selben Augenblick war das Gespraech
	// wieder weg. sipgo setzt das tag nur in den From-Kopf, den es selbst
	// baut; einen mitgegebenen laesst es unangetastet.
	von := sip.Uri{User: nutzerteil(name), Host: sipWirt}
	kennzeichen := sip.NewParams()
	kennzeichen.Add("tag", marke())
	kopf := []sip.Header{
		sip.NewHeader("Content-Type", "application/sdp"),
		&sip.FromHeader{
			DisplayName: name,
			Address:     von,
			Params:      kennzeichen,
		},
	}
	sitzung, err := b.dc.Invite(context.Background(), *kontakt, []byte(b.sdp()), kopf...)
	if err != nil {
		return err
	}
	// Eigener Kontext fuers Warten: wird er abgebrochen, schickt sipgo ein
	// CANCEL. Genau das braucht es, wenn der Anruf anderswo angenommen
	// oder vom Anrufer zurueckgezogen wird -- ohne CANCEL klingelte das
	// Telefon weiter.
	warteCtx, abbrechen := context.WithCancel(context.Background())
	b.mu.Lock()
	b.sitzung = sitzung
	b.abbruch = abbrechen
	b.angenommen = false
	b.mu.Unlock()

	go func() {
		err := sitzung.WaitAnswer(warteCtx, sipgo.AnswerOptions{
			OnResponse: func(res *sip.Response) error {
				// Jede Antwort ins Protokoll: ob das Telefon geklingelt
				// hat (180) oder abgelehnt (4xx), steht nur hier -- und
				// ohne das laesst sich "es klingelt nicht" nicht von "es
				// klingelt und niemand hebt ab" unterscheiden.
				melden("Telefon antwortet %d %s", res.StatusCode, res.Reason)
				if res.StatusCode == 200 {
					melden("Antwort des Telefons: %s", strings.Join(strings.Fields(string(res.Body())), " "))
					b.gegenstelleAusSDP(string(res.Body()))
				}
				return nil
			},
		})
		if err != nil {
			melden("nicht angenommen: %v", err)
			b.ereignis("aufgelegt")
			return
		}
		if err = sitzung.Ack(context.Background()); err != nil {
			melden("ACK: %v", err)
			return
		}
		b.mu.Lock()
		b.laeuft = true
		b.angenommen = true
		b.mu.Unlock()
		melden("angenommen, Ton laeuft")
		// Erst jetzt darf der Telegram-Anruf angenommen werden: bis hierhin
		// hat nur das Telefon geklingelt.
		b.ereignis("angenommen")

		<-sitzung.Context().Done()
		b.mu.Lock()
		b.laeuft = false
		b.mu.Unlock()
		melden("Telefon hat aufgelegt")
		b.ereignis("aufgelegt")
	}()
	return nil
}

// marke liefert ein SIP-tag: zufaellig, und je Dialog genau einmal.
func marke() string {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		return fmt.Sprintf("%x", time.Now().UnixNano())
	}
	return hex.EncodeToString(b)
}

// nutzerteil macht aus einem Namen etwas, das durch einen SIP-URI passt.
func nutzerteil(name string) string {
	var aus strings.Builder
	for _, r := range name {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9',
			r == '-', r == '_', r == '.':
			aus.WriteRune(r)
		case r == ' ':
			aus.WriteRune('.')
		}
	}
	s := aus.String()
	if s == "" {
		return "telegram"
	}
	return s
}

// klingelt und angenommen beantworten ein INVITE des Telefons -- den Weg,
// bei dem das Telefon waehlt und wir den Telegram-Anruf fuehren.
func (b *sipBruecke) klingelt() {
	b.mu.Lock()
	ss := b.serverSitzung
	b.mu.Unlock()
	if ss != nil {
		_ = ss.Respond(sip.StatusRinging, "Ringing", nil)
	}
}

func (b *sipBruecke) annehmen() error {
	b.mu.Lock()
	ss := b.serverSitzung
	b.mu.Unlock()
	if ss == nil {
		return fmt.Errorf("kein Gespraech vom Telefon")
	}
	// Erst hier, keinen Augenblick frueher: ein 200 OK sagt dem Telefon,
	// das Gespraech stehe. Kaeme es, bevor der Angerufene abgehoben hat,
	// redete man ins Leere.
	if err := ss.RespondSDP([]byte(b.sdp())); err != nil {
		return err
	}
	b.mu.Lock()
	b.laeuft = true
	b.angenommen = true
	b.mu.Unlock()
	melden("Telefon telefoniert, Ton laeuft")
	return nil
}

// auflegen beendet die SIP-Seite, wenn der Telegram-Anruf endet.
func (b *sipBruecke) auflegen(grund string) {
	// Zuerst die Server-Seite: ein Gespraech, das das Telefon gewaehlt hat.
	// Beide Wege raeumen unter derselben Sperre ab und setzen die Sitzung
	// auf nil, deshalb macht ein zweiter Aufruf nichts mehr -- und zweimal
	// aufgerufen wird hier oefter.
	b.mu.Lock()
	ss := b.serverSitzung
	stand := b.angenommen
	b.serverSitzung = nil
	b.mu.Unlock()
	if ss != nil {
		if stand {
			_ = ss.Bye(context.Background())
		} else {
			code, text := sip.StatusTemporarilyUnavailable, "Unavailable"
			switch grund {
			case "besetzt", "abgelehnt":
				code, text = sip.StatusBusyHere, "Busy Here"
			case "keine-antwort":
				code, text = sip.StatusRequestTimeout, "Request Timeout"
			}
			_ = ss.Respond(code, text, nil)
		}
		_ = ss.Close()
		b.mu.Lock()
		b.laeuft, b.angenommen = false, false
		b.mu.Unlock()
		melden("Gespraech mit dem Telefon beendet (%s)", grund)
	}

	b.mu.Lock()
	s := b.sitzung
	abbrechen := b.abbruch
	angenommen := b.angenommen
	b.laeuft, b.angenommen = false, false
	b.sitzung, b.abbruch = nil, nil
	b.mu.Unlock()
	if s == nil {
		return
	}
	if !angenommen {
		// Noch kein 200 OK: BYE scheiterte hier ("can not send as no invite
		// response present") und das Telefon klingelte weiter. Der Abbruch
		// des Wartekontexts loest stattdessen ein CANCEL aus.
		if abbrechen != nil {
			abbrechen()
		}
		melden("klingeln abgebrochen (%s)", grund)
		return
	}
	_ = s.Bye(context.Background())
}

func (b *sipBruecke) sdp() string {
	jetzt := time.Now().Unix()
	return strings.Join([]string{
		"v=0",
		fmt.Sprintf("o=- %d %d IN IP4 %s", jetzt, jetzt, sipHost),
		"s=Telegram",
		"c=IN IP4 " + sipHost,
		"t=0 0",
		fmt.Sprintf("m=audio %d RTP/AVP %d", rtpPort, nutzlastPCMU),
		fmt.Sprintf("a=rtpmap:%d PCMU/%d", nutzlastPCMU, sipRate),
		"a=ptime:20",
		"a=sendrecv",
		"",
	}, "\r\n")
}

func (b *sipBruecke) gegenstelleAusSDP(sdp string) {
	host, port := sipHost, 0
	for _, zeile := range strings.Split(sdp, "\n") {
		zeile = strings.TrimSpace(zeile)
		if strings.HasPrefix(zeile, "c=IN IP4 ") {
			host = strings.TrimSpace(strings.TrimPrefix(zeile, "c=IN IP4 "))
		}
		if strings.HasPrefix(zeile, "m=audio ") {
			if teile := strings.Fields(zeile); len(teile) > 1 {
				port, _ = strconv.Atoi(teile[1])
			}
		}
	}
	if port == 0 {
		return
	}
	b.mu.Lock()
	b.gegen = &net.UDPAddr{IP: net.ParseIP(host), Port: port}
	b.mu.Unlock()
}

// rtpLesen nimmt die Pakete des Telefons entgegen: G.711 auspacken, auf
// 16 kHz bringen, an den Tonprozess weitergeben.
//
// Hier haengt der Takt des Ganzen: jedes Paket sind 20 ms vom Mikrofon des
// Telefons, also genau zwei Rahmen fuer tgcalls. Der Tonprozess liest
// blockierend und wird damit von der Tonhardware des Telefons getaktet.
func (b *sipBruecke) rtpLesen() {
	puffer := make([]byte, 1500)
	for {
		n, von, err := b.rtp.ReadFromUDP(puffer)
		if err != nil {
			return
		}
		b.zielAktualisieren(von)
		b.mu.Lock()
		laeuft := b.laeuft
		b.letztesRTP = time.Now()
		b.mu.Unlock()
		if !laeuft {
			continue
		}
		var p rtp.Packet
		if err = p.Unmarshal(puffer[:n]); err != nil {
			continue
		}
		roh := make([]int16, len(p.Payload))
		var spitze int16
		for i, c := range p.Payload {
			w := muLawDekodieren(c)
			roh[i] = w
			if w < 0 {
				w = -w
			}
			if w > spitze {
				spitze = w
			}
		}
		if err := b.ton.schreiben(hoch(roh)); err != nil {
			melden("Ton an den Tonprozess: %v", err)
		}
		b.zaehlen(von, p.PayloadType, len(p.Payload), spitze)
	}
}

// rtpSenden schickt 20 ms (16 kHz) als ein G.711-Paket ans Telefon.
func (b *sipBruecke) rtpSenden(samples []int16) {
	b.mu.Lock()
	gegen, laeuft := b.gegen, b.laeuft
	b.mu.Unlock()
	if !laeuft || gegen == nil {
		return
	}
	acht := runter(samples)
	nutz := make([]byte, len(acht))
	for i, w := range acht {
		nutz[i] = muLawKodieren(w)
	}
	b.mu.Lock()
	b.sequenz++
	b.zeitmarke += uint32(len(acht))
	p := &rtp.Packet{
		Header: rtp.Header{
			Version:        2,
			PayloadType:    nutzlastPCMU,
			SequenceNumber: b.sequenz,
			Timestamp:      b.zeitmarke,
			SSRC:           b.ssrc,
		},
		Payload: nutz,
	}
	b.mu.Unlock()
	roh, err := p.Marshal()
	if err != nil {
		return
	}
	if _, err := b.rtp.WriteToUDP(roh, gegen); err == nil {
		b.mu.Lock()
		b.hinaus++
		b.mu.Unlock()
	}
}

// zaehlerZuruecksetzen -- die Zaehler gehoeren zum Gespraech, nicht zur
// Laufzeit. Aufrufer haelt die Sperre.
func (b *sipBruecke) zaehlerZuruecksetzen() {
	b.pakete, b.hinaus, b.spitze = 0, 0, 0
	b.gemeldet = time.Time{}
}

// zaehlen meldet einmal je Sekunde, was vom Telefon hereinkommt. Das erste
// Paket bekommt eine eigene Zeile: kommt gar nichts, steht im Protokoll
// nichts -- und genau das ist die Antwort auf "die Gegenseite hoert mich
// nicht".
func (b *sipBruecke) zaehlen(von *net.UDPAddr, art uint8, laenge int, spitze int16) {
	b.mu.Lock()
	b.pakete++
	if spitze > b.spitze {
		b.spitze = spitze
	}
	erstes := b.pakete == 1
	jetzt := time.Now()
	faellig := jetzt.Sub(b.gemeldet) >= time.Second
	anzahl, hoechste, hinaus := b.pakete, b.spitze, b.hinaus
	if erstes || faellig {
		b.gemeldet, b.spitze, b.hinaus = jetzt, 0, 0
	}
	b.mu.Unlock()
	if erstes {
		melden("erstes RTP vom Telefon: %s, Nutzlast %d, %d Bytes", von, art, laenge)
		return
	}
	if faellig {
		melden("Telefonmikrofon %d Pakete, Spitze %d, %d Pakete ans Telefon", anzahl, hoechste, hinaus)
	}
}

// zielAktualisieren merkt sich, wohin der Ton ans Telefon geht: dorthin,
// wo dessen eigene Pakete herkommen (symmetrisches RTP).
//
// Nicht dorthin, wohin die SDP-Antwort zeigt. Welche Adresse ein Endpunkt
// in die SDP schreibt, ist eine Behauptung; woher seine Pakete kommen, ist
// eine Tatsache. Am Mobilfunk stand dort die Adresse aus dem Carrier-NAT,
// und jedes Paket dorthin ging ueber den Router hinaus ins Netz.
func (b *sipBruecke) zielAktualisieren(von *net.UDPAddr) {
	if von == nil {
		return
	}
	b.mu.Lock()
	alt := b.gegen
	if alt != nil && alt.IP.Equal(von.IP) && alt.Port == von.Port {
		b.mu.Unlock()
		return
	}
	b.gegen = von
	b.mu.Unlock()
	if alt == nil {
		melden("Ton geht an %s (aus den Paketen des Telefons)", von)
		return
	}
	melden("Ton geht jetzt an %s statt an %s", von, alt)
}
