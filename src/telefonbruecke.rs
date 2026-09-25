//! Die Leitung zur SIP-Bruecke -- und damit zur Telefon-App.
//!
//! Der Anruf soll dem Telefon gehoeren, nicht uns. Klingeln am
//! Sperrbildschirm, Annehmen ohne die App zu oeffnen, Hoermuschel statt
//! Lautsprecher, Naeherungssensor, Lautstaerketasten: das alles bringt
//! Harmattan mit, wenn der Anruf durch seine eigene Anrufansicht laeuft.
//! Der Weg dorthin fuehrt ueber SIP, und dieses Modul ist der Draht zu
//! der Bruecke, die das uebersetzt.
//!
//! Die Reihenfolge bei einem eingehenden Anruf ist dabei wichtiger, als
//! sie aussieht:
//!
//!   1. Telegram meldet `Requested` -- jemand ruft an.
//!   2. Wir lassen das Telefon klingeln. Angenommen ist noch nichts.
//!   3. Der Nutzer hebt in der Anrufansicht ab; die Bruecke meldet
//!      `angenommen`.
//!   4. *Jetzt erst* nehmen wir den Telegram-Anruf an.
//!
//! Wer Schritt vier vorzieht, hat den Anruf angenommen, waehrend das
//! Telefon noch klingelt: die Gegenstelle redet dann ins Leere, bis
//! jemand abhebt.

use std::io::ErrorKind;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::Lage;

/// Wo das Bruecken-Programm liegt.
pub const BRUECKENPROGRAMM: &str = "/opt/drahtpost/drahtpost-bruecke";

static HINAUS: OnceLock<mpsc::UnboundedSender<String>> = OnceLock::new();
/// Hat sich ein Telefon an der Bruecke angemeldet? Nur dann kann sie
/// klingeln -- sonst telefoniert man wie zuvor ueber PulseAudio.
static TELEFON_DA: AtomicBool = AtomicBool::new(false);
/// Steht gerade ein Gespraech mit der Telefon-App? Davon haengt ab, wo
/// der Ton hingeht: nur dann kann der Tonprozess ihn an die Bruecke
/// geben, sonst wartete er auf Rahmen, die nie kommen.
static GESPRAECH_STEHT: AtomicBool = AtomicBool::new(false);

pub fn socket() -> std::path::PathBuf {
    crate::datenverzeichnis().join("bruecke.sock")
}

/// Klingelt bei diesem Gespraech das Telefon?
pub fn telefon_da() -> bool {
    TELEFON_DA.load(Ordering::Relaxed)
}

/// Fuehrt die Telefon-App gerade das Gespraech?
pub fn im_gespraech() -> bool {
    GESPRAECH_STEHT.load(Ordering::Relaxed)
}

/// Das Telefon klingeln lassen. Gibt es keines, sagt das Ergebnis es --
/// dann bleibt es beim alten Weg ueber PulseAudio.
pub fn klingeln(name: &str) -> bool {
    if !telefon_da() {
        eprintln!("== kein Telefon an der Bruecke -- Anruf laeuft ueber PulseAudio");
        return false;
    }
    GESPRAECH_STEHT.store(false, Ordering::Relaxed);
    sagen(format!("klingeln {name}"));
    true
}

/// Die SIP-Seite beenden. Schadet nie: steht dort nichts, tut die Bruecke
/// nichts.
pub fn auflegen(grund: &str) {
    GESPRAECH_STEHT.store(false, Ordering::Relaxed);
    if HINAUS.get().is_some() {
        sagen(format!("auflegen {grund}"));
    }
}

/// Einen Befehl an die Bruecke schicken. Nicht blockierend: die Antwort
/// interessiert hier nicht, die Ereignisse kommen ohnehin von selbst.
pub fn sagen(befehl: impl Into<String>) {
    let b = befehl.into();
    match HINAUS.get() {
        Some(s) => {
            let _ = s.send(b);
        }
        None => eprintln!("⚠ Bruecke: noch keine Leitung fuer {b:?}"),
    }
}

/// Die Leitung aufbauen und halten. Laeuft, solange der Daemon laeuft.
pub fn starten(lage: Arc<Lage>) {
    let (sender, mut empfang) = mpsc::unbounded_channel::<String>();
    if HINAUS.set(sender).is_err() {
        return;
    }
    tokio::task::spawn(async move {
        loop {
            let strom = match verbinden().await {
                Some(s) => s,
                None => {
                    // Kein Grund zur Eile: ohne Bruecke laeuft alles wie
                    // zuvor, nur eben ueber den Lautsprecher.
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                    continue;
                }
            };
            eprintln!("== Bruecke verbunden");
            let (lesen, mut schreiben) = strom.into_split();
            let mut zeilen = BufReader::new(lesen).lines();
            loop {
                tokio::select! {
                    zeile = zeilen.next_line() => match zeile {
                        Ok(Some(z)) => antwort_lesen(&lage, z.trim()).await,
                        _ => break,
                    },
                    befehl = empfang.recv() => match befehl {
                        Some(b) => {
                            eprintln!("== an die Bruecke: {b}");
                            if schreiben.write_all(format!("{b}\n").as_bytes()).await.is_err() {
                                break;
                            }
                        }
                        None => return,
                    },
                }
            }
            eprintln!("⚠ Bruecke: Leitung weg");
            TELEFON_DA.store(false, Ordering::Relaxed);
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
}

/// Verbinden -- und die Bruecke starten, falls sie nicht laeuft.
async fn verbinden() -> Option<tokio::net::UnixStream> {
    let pfad = socket();
    for versuch in 0..2 {
        match tokio::net::UnixStream::connect(&pfad).await {
            Ok(s) => return Some(s),
            Err(e) if versuch == 0 && std::path::Path::new(BRUECKENPROGRAMM).exists() => {
                if e.kind() != ErrorKind::NotFound && e.kind() != ErrorKind::ConnectionRefused {
                    return None;
                }
                starten_lassen();
                for _ in 0..40 {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    if pfad.exists() {
                        break;
                    }
                }
            }
            Err(_) => return None,
        }
    }
    None
}

fn starten_lassen() {
    let _ = std::fs::remove_file(socket());
    eprintln!("== starte die SIP-Bruecke");
    // Ihre Ausgabe gehoert in eine Datei. Beim Tonprozess hatte sie einmal
    // nach /dev/null gezeigt, und als er abstuerzte, blieb keine Erklaerung
    // uebrig -- nur zwei Zombies in der Prozessliste.
    let protokoll = crate::datenverzeichnis().join("bruecke.log");
    let hin = std::fs::OpenOptions::new().create(true).append(true).open(&protokoll);
    let (aus, fehler) = match hin {
        Ok(f) => match f.try_clone() {
            Ok(g) => (Stdio::from(f), Stdio::from(g)),
            Err(_) => (Stdio::null(), Stdio::null()),
        },
        Err(_) => (Stdio::null(), Stdio::null()),
    };
    match std::process::Command::new(BRUECKENPROGRAMM).stdout(aus).stderr(fehler).spawn() {
        Ok(mut kind) => {
            std::thread::spawn(move || {
                if let Ok(stand) = kind.wait() {
                    if !stand.success() {
                        eprintln!("⚠ Bruecke endete: {stand}");
                    }
                }
            });
        }
        Err(e) => eprintln!("⚠ Bruecke startet nicht: {e}"),
    }
}

async fn antwort_lesen(lage: &Arc<Lage>, zeile: &str) {
    let Some(rest) = zeile.strip_prefix("ereignis ") else {
        // Antworten auf Befehle ("ok", "fehler ...") stehen im Protokoll
        // und sonst nirgends -- gehandelt wird auf Ereignisse.
        if !zeile.is_empty() && zeile != "ok" {
            eprintln!("== Bruecke: {zeile}");
        }
        // Scheitert das Klingeln, ist kein Telefon mehr da -- der
        // naechste Anruf soll dann gar nicht erst darauf warten, sondern
        // gleich den alten Weg nehmen.
        if zeile.starts_with("fehler") {
            TELEFON_DA.store(false, Ordering::Relaxed);
        }
        return;
    };
    let (wort, wovon) = match rest.split_once(' ') {
        Some((w, r)) => (w, r.trim()),
        None => (rest, ""),
    };
    eprintln!("== Bruecke meldet: {rest}");
    match wort {
        "registriert" => TELEFON_DA.store(true, Ordering::Relaxed),

        // Der Nutzer hat in der Anrufansicht abgehoben. Erst jetzt darf
        // der Telegram-Anruf angenommen werden.
        "angenommen" => {
            GESPRAECH_STEHT.store(true, Ordering::Relaxed);
            if let Err(e) = crate::anrufweg::abheben(lage).await {
                eprintln!("⚠ Abheben nach dem Telefon: {e}");
            }
        }

        // Aufgelegt oder abgelehnt -- beides endet den Telegram-Anruf.
        // Ist dort schon nichts mehr, macht das nichts: beenden() raeumt
        // auch dann auf und meldet nur, dass es nichts zu tun gab.
        "aufgelegt" => {
            GESPRAECH_STEHT.store(false, Ordering::Relaxed);
            // Abgelehnt oder nicht abgehoben? Der Anrufer sieht den
            // Unterschied, und er entscheidet, ob er es gleich nochmal
            // versucht.
            let grund = if wovon == "abgelehnt" { "busy" } else { "hangup" };
            if lage.gespraech.lock().await.is_some() {
                if let Err(e) = crate::anrufweg::beenden(lage, grund).await {
                    eprintln!("⚠ Auflegen nach dem Telefon: {e}");
                }
                lage.melden(json!({"event": "call_ended", "data": {"reason": "hangup"}}));
            }
        }

        // Das Telefon waehlt ueber unser SIP-Konto. Dafuer muessten wir
        // eine Telefonnummer einem Telegram-Konto zuordnen; das kann
        // dieser Stand noch nicht, und stillschweigend den Falschen
        // anzurufen waere schlimmer als eine sichtbare Absage.
        "waehlt" => {
            eprintln!("⚠ Waehlen vom Telefon aus ({wovon}) kann die Bruecke noch nicht");
            sagen("auflegen nicht-unterstuetzt");
        }

        _ => {}
    }
}
