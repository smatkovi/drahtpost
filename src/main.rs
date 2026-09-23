//! Drahtpost -- der Telegram-Daemon fuer Harmattan, in Rust.
//!
//! Er ersetzt telegram_daemon.py Zeile fuer Zeile in dem, was nach aussen
//! sichtbar ist: derselbe Unix-Socket, dieselben Befehle, dieselben
//! Feldnamen. Weder die Qt-Oberflaeche von PyTeleGram noch die
//! Nachrichtenbruecke merken den Unterschied -- sie sehen nur, dass
//! statt 55 MB Python ein paar Megabyte laufen.
//!
//! Warum ueberhaupt: auf einem Geraet mit 1 GB, von dem nach dem
//! Hochfahren 230 MB frei sind, ist ein Telethon-Daemon je Protokoll
//! nicht zu bezahlen. grammers spricht MTProto in reinem Rust -- kein
//! TDLib, kein C++, kein Python.
//!
//! Was hier NICHT drin ist: Sprach- und Videoanrufe. Die laufen bei
//! Telegram ueber tgcalls auf WebRTC-Basis, und das ist ein eigenes
//! Vorhaben.

mod befehle;
mod formen;
mod sitzung;
mod sperre;
mod stumm;
mod verzeichnis;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use grammers_client::{Client, Config, InitParams, Update};
use grammers_tl_types as tl;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Mutex};

use stumm::Stummliste;
use verzeichnis::Verzeichnis;

/// Die oeffentlichen Zugangsdaten von Telegram Desktop -- dieselben, die
/// der Python-Daemon benutzt. Das ist wichtig: der uebernommene
/// Schluessel gehoert zu einer Sitzung, die damit angelegt wurde.
pub const API_ID: i32 = 2040;
pub const API_HASH: &str = "b18441a1ff607e10a989891a5462e627";

pub fn datenverzeichnis() -> PathBuf {
    let heim = std::env::var("HOME").unwrap_or_else(|_| "/home/user".into());
    PathBuf::from(heim).join(".pytelegram")
}

/// Alles, was die Befehle brauchen.
pub struct Lage {
    pub client: Client,
    pub verzeichnis: Verzeichnis,
    /// Welche Chats stummgeschaltet sind. Ihre Nachrichten gehen
    /// weiterhin an die Oberflaeche, aber mit einer Marke, an der die
    /// Bruecke sie erkennt und aus der Nachrichten-App haelt.
    pub stumm: Stummliste,
    /// Zwischen send_phone und send_code liegt ein Token, das der Server
    /// vergeben hat. Ohne es ist der Code wertlos.
    pub anmeldung: Mutex<sitzung::Anmeldung>,
    pub einstellungen: Mutex<Value>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = lauf().await {
        eprintln!("✗ {e}");
        std::process::exit(1);
    }
}

/// Wie oft und wie lange grammers von sich aus neu verbindet.
///
/// Die Vorgabe ist NoReconnect -- ein einziger Abriss, und der Daemon
/// redet nie wieder mit Telegram, waehrend er scheinbar weiterlaeuft.
/// Auf diesem Geraet ist der Abriss der Normalfall: das icd2-Signal, auf
/// das die Bruecke horcht, gibt es genau deshalb.
static NEUVERBINDEN: grammers_mtsender::FixedReconnect = grammers_mtsender::FixedReconnect {
    attempts: usize::MAX,
    delay: Duration::from_secs(5),
};

async fn lauf() -> Result<(), String> {
    let daten = datenverzeichnis();
    std::fs::create_dir_all(daten.join("downloads")).ok();
    std::fs::create_dir_all(daten.join("cache/avatars")).ok();

    // Vor allem anderen: nur eine Instanz.
    sperre::nehmen(&daten)?;
    protokoll_umlenken(&daten);

    // Beim Einschalten ist das Netz oft noch nicht da. Aufgeben waere
    // falsch -- die Bruecke wuerde das als gescheiterten Start sehen und
    // beim naechsten Versuch einen zweiten Daemon starten.
    let mut versuch = 0u32;
    let client = loop {
        versuch += 1;
        eprintln!("== verbinde (Versuch {versuch})");
        // Die Sitzung wird je Versuch neu geholt: Session ist nicht
        // kopierbar, und beim ersten Mal steckt hier die Uebernahme aus
        // Telethon. Danach wird sie nur noch geladen.
        let sitzung = sitzung::vorbereiten(&daten)?;
        let ergebnis = Client::connect(Config {
            session: sitzung,
            api_id: API_ID,
            api_hash: API_HASH.to_string(),
            params: InitParams {
                // Updates nachholen, die waehrend des Ausschaltens
                // anfielen -- sonst fehlen genau die Nachrichten, die
                // ueber Nacht kamen.
                catch_up: true,
                reconnection_policy: &NEUVERBINDEN,
                ..Default::default()
            },
        })
        .await;
        match ergebnis {
            Ok(c) => break c,
            Err(e) => {
                eprintln!("⚠ Verbindung: {e}");
                let warten = std::cmp::min(60, 5 * versuch) as u64;
                tokio::time::sleep(Duration::from_secs(warten)).await;
            }
        }
    };

    let angemeldet = client.is_authorized().await.unwrap_or(false);
    eprintln!("== angemeldet: {angemeldet}");

    let lage = Arc::new(Lage {
        verzeichnis: Verzeichnis::laden(&daten),
        stumm: Stummliste::laden(&daten),
        anmeldung: Mutex::new(sitzung::Anmeldung::neu(angemeldet)),
        einstellungen: Mutex::new(befehle::einstellungen_laden(&daten)),
        client: client.clone(),
    });

    // Der Socket. Ein altes, verwaistes Exemplar liegt nach einem
    // Absturz noch da und wuerde jeden Verbindungsversuch mit
    // "connection refused" beantworten -- also weg damit, aber nur wenn
    // wirklich niemand antwortet.
    let socketpfad = daten.join("daemon.sock");
    if UnixStream::connect(&socketpfad).await.is_ok() {
        return Err(format!("es laeuft schon ein Daemon auf {}", socketpfad.display()));
    }
    let _ = std::fs::remove_file(&socketpfad);
    let horcher = UnixListener::bind(&socketpfad).map_err(|e| format!("bind: {e}"))?;
    setze_rechte(&socketpfad);
    eprintln!("== Socket: {}", socketpfad.display());

    // Ereignisse gehen an alle offenen Verbindungen -- die Oberflaeche
    // und die Bruecke haengen gleichzeitig dran.
    let (ruf, _) = broadcast::channel::<String>(256);

    // Die Sitzung regelmaessig auf die Platte.
    //
    // grammers sichert nichts von selbst -- Telethon schrieb seine
    // SQLite-Datei laufend mit. Ohne das hier ginge nach jedem Start der
    // Stand der Updates verloren (catch_up waere wirkungslos), und die
    // Schluessel, die fuer Downloads mit anderen Rechenzentren
    // ausgehandelt werden, muessten jedes Mal neu ausgehandelt werden.
    {
        let client = client.clone();
        let pfad = daten.join("grammers.session");
        tokio::spawn(async move {
            let mut takt = tokio::time::interval(Duration::from_secs(30));
            loop {
                takt.tick().await;
                let _ = client.session().save_to_file(&pfad);
            }
        });
    }

    // Updates einsammeln.
    {
        let lage = lage.clone();
        let ruf = ruf.clone();
        tokio::spawn(async move {
            loop {
                match lage.client.next_update().await {
                    Ok(u) => {
                        for zeile in ereignis_zeilen(&lage, u).await {
                            let _ = ruf.send(zeile);
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠ Update-Strom: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    // Das Verzeichnis einmal auffrischen, damit send_message auch dann
    // einen Chat findet, wenn die Oberflaeche noch keine Dialoge geholt
    // hat. Im Hintergrund: es darf den Start nicht aufhalten.
    //
    // Dasselbe gilt fuer die Stummschaltungen: ohne sie wuesste der
    // Daemon bei einem frisch installierten Drahtpost nicht, welche
    // Gruppen der Benutzer stumm gestellt hat, und die Bruecke bekaeme
    // sie alle. Ein volles Verzeichnis heisst dabei nicht, dass auch die
    // Stummliste steht -- sie ist neuer als das Verzeichnis.
    if angemeldet {
        let lage = lage.clone();
        tokio::spawn(async move {
            if lage.verzeichnis.leer() || !lage.stumm.bekannt() {
                eprintln!("== Verzeichnis oder Stummliste fehlt, hole Dialoge");
                let _ = befehle::dialoge_holen(&lage, 400, 0).await;
            }
        });
    }

    loop {
        let (strom, _) = horcher.accept().await.map_err(|e| format!("accept: {e}"))?;
        let lage = lage.clone();
        let mut ereignisse = ruf.subscribe();
        tokio::spawn(async move {
            let (lesen, schreiben) = strom.into_split();
            let schreiben = Arc::new(Mutex::new(schreiben));

            // Ereignisse nebenher hinausschieben.
            let s = schreiben.clone();
            let ausgabe = tokio::spawn(async move {
                loop {
                    match ereignisse.recv().await {
                        Ok(zeile) => {
                            let mut w = s.lock().await;
                            if w.write_all(zeile.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                        // Ein Schwall -- etwa beim Nachholen nach der
                        // Nacht -- laesst den Empfaenger zurueckfallen.
                        // Das ist kein Grund, den Ereignisstrom dieser
                        // Verbindung fuer immer zu schliessen; genau das
                        // taete ein `while let Ok(..)`.
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            eprintln!("⚠ {n} Ereignisse uebersprungen");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Fragen der Reihe nach beantworten. Der Python-Daemon machte
            // es genauso, und die Bruecke verlaesst sich darauf: sie
            // ordnet Antworten nach Reihenfolge zu, nicht nach Nummern.
            let mut zeilen = BufReader::new(lesen).lines();
            while let Ok(Some(zeile)) = zeilen.next_line().await {
                let Ok(frage) = serde_json::from_str::<Value>(&zeile) else {
                    continue;
                };
                let antwort = befehle::behandeln(&lage, &frage).await;
                let text = format!("{antwort}\n");
                let mut w = schreiben.lock().await;
                if w.write_all(text.as_bytes()).await.is_err() {
                    break;
                }
            }
            ausgabe.abort();
        });
    }
}

/// Die Bruecke startet Drahtpost mit geschlossener Fehlerausgabe. Damit
/// im Fehlerfall trotzdem etwas nachzulesen ist, geht sie in eine Datei
/// neben der Sitzung -- nicht nach /tmp: das ist hier ein tmpfs mit 4 MB,
/// und ein volles /tmp legt mehr lahm als ein fehlendes Protokoll.
fn protokoll_umlenken(daten: &std::path::Path) {
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn isatty(fd: i32) -> i32;
        fn dup2(alt: i32, neu: i32) -> i32;
    }
    if unsafe { isatty(2) } == 1 {
        return; // Von Hand gestartet: auf dem Bildschirm ist es besser aufgehoben.
    }
    let pfad = daten.join("drahtpost.log");
    // Nicht anwachsen lassen.
    if std::fs::metadata(&pfad).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&pfad, daten.join("drahtpost.log.alt"));
    }
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&pfad) {
        unsafe {
            dup2(f.as_raw_fd(), 1);
            dup2(f.as_raw_fd(), 2);
        }
        std::mem::forget(f);
    }
}

/// 0600 wie beim Python-Daemon.
fn setze_rechte(pfad: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(pfad, std::fs::Permissions::from_mode(0o600));
}

/// Ein Update in die Zeilen uebersetzen, die der Python-Daemon gesendet
/// haette.
async fn ereignis_zeilen(lage: &Arc<Lage>, u: Update) -> Vec<String> {
    let mut aus = Vec::new();
    match u {
        Update::NewMessage(m) => {
            lage.verzeichnis.merken(&m.chat().pack());
            if let Some(s) = m.sender() {
                lage.verzeichnis.merken(&s.pack());
            }
            aus.push(zeile(json!({
                "event": "new_message",
                "data": gemerkt(lage, &m),
            })));
        }
        Update::MessageEdited(m) => {
            lage.verzeichnis.merken(&m.chat().pack());
            aus.push(zeile(json!({
                "event": "message_edited",
                "data": gemerkt(lage, &m),
            })));
        }
        Update::MessageDeleted(d) => {
            aus.push(zeile(json!({
                "event": "message_deleted",
                "data": {"ids": d.messages()},
            })));
        }
        // Stummschalten und wieder lautstellen kommt als rohes Update
        // herein -- grammers hat dafuer keine eigene Spielart. Ohne das
        // hier waere die Tabelle so alt wie der letzte Dialogdurchlauf,
        // und eine gerade stummgeschaltete Gruppe laendete den ganzen Tag
        // weiter in der Nachrichten-App.
        Update::Raw(tl::enums::Update::NotifySettings(u)) => {
            let bis = stumm::bis_aus_einstellungen(&u.notify_settings);
            match &u.peer {
                tl::enums::NotifyPeer::Peer(p) => {
                    let kennung = stumm::kennung_aus_peer(&p.peer);
                    eprintln!("== stumm {kennung} bis {bis}");
                    lage.stumm.setzen(kennung, bis);
                    lage.stumm.sichern();
                }
                // notifyUsers/notifyChats/notifyBroadcasts sind die
                // Voreinstellung einer ganzen Gattung. Die Dialoge, die
                // ihr folgen, tragen selbst kein mute_until -- das waere
                // ein eigener Weg, und bis dahin bleibt es beim
                // Einzelstand.
                andere => eprintln!("== Benachrichtigungen fuer {andere:?} geaendert"),
            }
        }
        _ => {}
    }
    aus
}

/// Die Nachricht in ihrer ueberlieferten Form, dazu zwei Felder, die der
/// Python-Daemon nicht hatte: ob der Chat stumm ist und ob der Benutzer
/// darin erwaehnt wird. Die Bruecke entscheidet damit, ob die Nachricht
/// in die Nachrichten-App gehoert; die Oberflaeche von PyTeleGram liest
/// unbekannte Felder nicht und bekommt weiterhin alles.
fn gemerkt(lage: &Arc<Lage>, m: &grammers_client::types::Message) -> Value {
    let mut v = formen::nachricht(m);
    let kennung = verzeichnis::markiert(&m.chat().pack());
    v["muted"] = json!(lage.stumm.stumm(kennung));
    // In einer stummen Gruppe meldet Telegram trotzdem, wenn man selbst
    // gemeint ist -- Erwaehnung oder Antwort auf die eigene Nachricht.
    v["mentioned"] = json!(m.mentioned());
    v
}

fn zeile(v: Value) -> String {
    format!("{v}\n")
}
