//! Eine exklusive Sperre auf dem Datenverzeichnis.
//!
//! Der Anlass ist ein beobachteter Fehler, nicht eine Vorsichtsmassnahme:
//! bei Signal warfen sich zwei Dienste mit derselben Geraetekennung
//! gegenseitig vom Websocket, beim WhatsApp-Port schrieb eine Instanz
//! ihren leeren Nachrichtenspeicher ueber den vollen.
//!
//! Hier ist die Stelle besonders eng: die Bruecke startet Drahtpost,
//! wenn sich der Socket nicht verbinden laesst, und der Socket entsteht
//! erst nach dem Verbinden mit Telegram -- ueber GPRS sind das zehn
//! Sekunden und mehr. In dieser Luecke waere ein zweiter Start moeglich,
//! und der wuerde dem ersten den Socket unter den Fuessen wegnehmen. Die
//! Sperre wird deshalb als allererstes genommen, vor allem anderen.
//!
//! flock ist hier das richtige Mittel: es haengt am Dateideskriptor und
//! verschwindet mit dem Prozess, auch wenn er abstuerzt. Kein verwaister
//! Sperreintrag, um den sich jemand kuemmern muesste.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// Offen gehalten, solange der Prozess laeuft -- mit dem Deskriptor faellt
/// die Sperre.
static mut SPERRDATEI: Option<std::fs::File> = None;

/// Belegt das Datenverzeichnis. Gelingt es nicht, laeuft schon eine
/// andere Instanz und diese hier soll sich beenden.
pub fn nehmen(verzeichnis: &Path) -> Result<(), String> {
    let pfad = verzeichnis.join(".drahtpost.lock");
    let datei = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&pfad)
    {
        Ok(f) => f,
        Err(e) => {
            // Kein Verzeichnis, kein Platz -- lieber weiterlaufen als gar
            // nicht starten. Die Sperre ist eine Absicherung, keine
            // Voraussetzung.
            eprintln!("⚠ Sperre nicht anlegbar ({e}) - laufe ohne");
            return Ok(());
        }
    };

    // LOCK_EX | LOCK_NB
    let ergebnis = unsafe { flock(datei.as_raw_fd(), 2 | 4) };
    if ergebnis != 0 {
        return Err(format!("eine andere Instanz haelt {}", pfad.display()));
    }

    let mut datei = datei;
    let _ = writeln!(datei, "{}", std::process::id());
    unsafe {
        SPERRDATEI = Some(datei);
    }
    Ok(())
}

extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}
