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
mod verzeichnis;

use std::path::PathBuf;
use std::sync::Arc;

use grammers_client::{Client, Config, InitParams, Update};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Mutex};

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

async fn lauf() -> Result<(), String> {
    let daten = datenverzeichnis();
    std::fs::create_dir_all(daten.join("downloads")).ok();
    std::fs::create_dir_all(daten.join("cache/avatars")).ok();

    // Erst die Sitzung: ohne sie waere jeder Start eine Neuanmeldung.
    let sitzung = sitzung::vorbereiten(&daten)?;

    eprintln!("== verbinde");
    let client = Client::connect(Config {
        session: sitzung,
        api_id: API_ID,
        api_hash: API_HASH.to_string(),
        params: InitParams {
            // Updates nachholen, die waehrend des Ausschaltens anfielen --
            // sonst fehlen genau die Nachrichten, die waehrend der Nacht
            // kamen.
            catch_up: true,
            ..Default::default()
        },
    })
    .await
    .map_err(|e| format!("Verbindung: {e}"))?;

    let angemeldet = client.is_authorized().await.unwrap_or(false);
    eprintln!("== angemeldet: {angemeldet}");

    let lage = Arc::new(Lage {
        verzeichnis: Verzeichnis::laden(&daten),
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
    let (ruf, _) = broadcast::channel::<String>(64);

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
    if angemeldet {
        let lage = lage.clone();
        tokio::spawn(async move {
            if lage.verzeichnis.leer() {
                eprintln!("== Verzeichnis ist leer, hole Dialoge");
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
                while let Ok(zeile) = ereignisse.recv().await {
                    let mut w = s.lock().await;
                    if w.write_all(zeile.as_bytes()).await.is_err() {
                        break;
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
                "data": formen::nachricht(&m),
            })));
        }
        Update::MessageEdited(m) => {
            lage.verzeichnis.merken(&m.chat().pack());
            aus.push(zeile(json!({
                "event": "message_edited",
                "data": formen::nachricht(&m),
            })));
        }
        Update::MessageDeleted(d) => {
            aus.push(zeile(json!({
                "event": "message_deleted",
                "data": {"ids": d.messages()},
            })));
        }
        _ => {}
    }
    aus
}

fn zeile(v: Value) -> String {
    format!("{v}\n")
}
