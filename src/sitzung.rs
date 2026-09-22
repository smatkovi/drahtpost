//! Die Sitzung -- und die einmalige Uebernahme von Telethon.
//!
//! Der Auth-Key einer Telegram-Sitzung haengt am Rechenzentrum, nicht an
//! der Bibliothek, die ihn ausgehandelt hat. Telethon legt ihn in
//! session.session ab (SQLite, Tabelle `sessions`, Spalten dc_id,
//! server_address, port, auth_key). Dieselben 256 Bytes in eine
//! grammers-Sitzung gelegt, und der Benutzer bleibt angemeldet.
//!
//! Auf dem Geraet nachgemessen: Uebernahme, `is_authorized() == true`,
//! 861 Dialoge -- ohne eine einzige SMS.
//!
//! Eine Falle dabei: Client::connect waehlt sein Rechenzentrum aus
//! `session.get_user()`. Ist dort niemand eingetragen, nimmt es das
//! Vorgabe-DC 2, legt dort einen frischen Schluessel an und der Server
//! antwortet mit AUTH_KEY_UNREGISTERED. Es muss also ein Benutzer
//! eingetragen sein, bevor verbunden wird -- die Kennung selbst wird
//! danach berichtigt.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;

use grammers_session::Session;

/// Wo die Anmeldung gerade steht. Der Python-Daemon fuehrte dasselbe als
/// Zeichenkette, und die Oberflaeche fragt sie mit get_auth_state ab.
pub struct Anmeldung {
    pub zustand: String,
    pub telefon: String,
    pub anmeldemarke: Option<grammers_client::types::LoginToken>,
    pub passwortmarke: Option<grammers_client::types::PasswordToken>,
}

impl Anmeldung {
    pub fn neu(angemeldet: bool) -> Self {
        Self {
            zustand: if angemeldet { "ready".into() } else { "need_phone".into() },
            telefon: String::new(),
            anmeldemarke: None,
            passwortmarke: None,
        }
    }
}

/// Die Sitzung besorgen: laden, oder aus Telethon uebernehmen, oder leer.
pub fn vorbereiten(daten: &Path) -> Result<Session, String> {
    let eigen = daten.join("grammers.session");
    if eigen.exists() {
        match Session::load_file(&eigen) {
            Ok(s) if s.signed_in() => {
                eprintln!("== Sitzung geladen");
                return Ok(s);
            }
            Ok(_) => eprintln!("⚠ Sitzung ohne Benutzer -- versuche Uebernahme"),
            Err(e) => eprintln!("⚠ Sitzung unlesbar ({e}) -- versuche Uebernahme"),
        }
    }

    let telethon = daten.join("session.session");
    if telethon.exists() {
        match uebernehmen(&telethon) {
            Ok(s) => {
                // save_to_file oeffnet nur, es legt nicht an.
                let _ = std::fs::File::create(&eigen);
                s.save_to_file(&eigen).map_err(|e| format!("Sitzung schreiben: {e}"))?;
                eprintln!("== Telethon-Schluessel uebernommen");
                return Ok(s);
            }
            Err(e) => eprintln!("⚠ Uebernahme gescheitert: {e}"),
        }
    }

    // Nichts da -- der Benutzer meldet sich ueber die Oberflaeche an.
    eprintln!("== keine Sitzung, Anmeldung noetig");
    let _ = std::fs::File::create(&eigen);
    Ok(Session::new())
}

fn uebernehmen(telethon: &Path) -> Result<Session, String> {
    let db = rusqlite::Connection::open_with_flags(
        telethon,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| e.to_string())?;

    let (dc, adresse, port, schluessel): (i32, String, u16, Vec<u8>) = db
        .query_row(
            "select dc_id, server_address, port, auth_key from sessions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| e.to_string())?;

    if schluessel.len() != 256 {
        return Err(format!("Schluessel ist {} Bytes lang", schluessel.len()));
    }
    let mut fest = [0u8; 256];
    fest.copy_from_slice(&schluessel);

    let ziel: SocketAddr = format!("{adresse}:{port}")
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("keine Adresse")?;

    let s = Session::new();
    s.insert_dc(dc, ziel, fest);
    // Platzhalter: er waehlt nur das Rechenzentrum. Nach dem ersten
    // get_me steht die richtige Kennung darin.
    s.set_user(0, dc, false);
    Ok(s)
}

/// Nach einem erfolgreichen get_me: die eigene Kennung festschreiben.
pub fn benutzer_festhalten(client: &grammers_client::Client, daten: &Path, kennung: i64, dc: i32) {
    let s = client.session();
    s.set_user(kennung, dc, false);
    // Immer schreiben, auch wenn die Kennung schon stimmte: nach einer
    // frischen Anmeldung steckt der neu ausgehandelte Schluessel in
    // derselben Sitzung, und ohne dieses Sichern waere er beim naechsten
    // Start weg -- der Benutzer muesste sich wieder anmelden.
    let eigen = daten.join("grammers.session");
    let _ = std::fs::OpenOptions::new().create(true).write(true).open(&eigen);
    if let Err(e) = s.save_to_file(&eigen) {
        eprintln!("⚠ Sitzung nicht gesichert: {e}");
    }
}
