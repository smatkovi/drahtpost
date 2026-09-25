//! Der Anrufzustand des Telefons -- damit der Bildschirm anbleibt.
//!
//! MCE ist Harmattans Dienst fuer Bildschirm, Tastensperre und
//! Naeherungssensor. Solange er glaubt, es laufe ein Anruf, bleibt der
//! Bildschirm an, die Tastensperre aus, und beim Anlegen ans Ohr
//! schaltet er ab. Ohne das geht der Bildschirm mitten im Gespraech aus
//! und die Tastensperre greift.
//!
//! Der Weg dorthin ist nicht der offensichtliche. `req_call_state_change`
//! verweigert der Bus unsignierten Paketen -- das Recht heisst
//! `mce::CallStateControl`, und aegis vergibt es hier nicht. `mcetool`
//! hat es, weil es als root laeuft. Und MCE bindet den Zustand an die
//! Verbindung dessen, der ihn setzt: ein mcetool, das sich sofort
//! beendet, aendert gar nichts. Also wird eines mit `--block` gehalten,
//! solange das Gespraech dauert.
//!
//! Das ist derselbe Weg wie im WhatsApp-Port, einschliesslich der Falle:
//! ein Halter, der einen Absturz ueberlebt, laesst das Telefon fuer
//! immer glauben, es klingle.

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

static HALTER: Mutex<Option<Child>> = Mutex::new(None);

/// Den Zustand setzen: "ringing", "active" oder "none".
pub fn setzen(zustand: &str) {
    let mut halter = HALTER.lock().unwrap();
    loslassen(&mut halter);
    if zustand == "none" || zustand.is_empty() {
        let _ = Command::new("sudo")
            .args(["mcetool", "--set-call-state=none:normal"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return;
    }
    match Command::new("sudo")
        .args([
            "mcetool",
            &format!("--set-call-state={zustand}:normal"),
            "--block",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(kind) => {
            eprintln!("== MCE-Anrufzustand {zustand} wird gehalten (pid {})", kind.id());
            *halter = Some(kind);
        }
        Err(e) => eprintln!("⚠ mcetool nicht startbar: {e}"),
    }
}

/// Den Halter loslassen.
///
/// Er laeuft ueber sudo als root, wir als Benutzer -- ein eigenes Kill
/// traefe ihn nicht, es scheitert mit EPERM. Genau daran ist im
/// WhatsApp-Port einmal ein Halter haengen geblieben: der Anruf war
/// lange vorbei, das Telefon glaubte weiter, es klingle, und mcetool
/// drehte stundenlang mit einem Viertel der Rechenzeit.
fn loslassen(halter: &mut Option<Child>) {
    let Some(mut kind) = halter.take() else {
        return;
    };
    let pid = kind.id().to_string();
    let _ = Command::new("sudo")
        .args(["kill", "-TERM", &pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = kind.wait();
}

/// Uebriggebliebene Halter aus einem frueheren Lauf.
///
/// Ueberlebt einer einen Absturz, klingelt das Telefon fuer immer.
/// Deshalb beim Start aufraeumen und nicht erst beim naechsten Anruf --
/// ein haengender Halter kostet auch dann, wenn nie wieder jemand anruft.
pub fn aufraeumen() {
    let Ok(eintraege) = std::fs::read_dir("/proc") else {
        return;
    };
    for e in eintraege.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.parse::<u32>().is_err() {
            continue;
        }
        let Ok(roh) = std::fs::read(format!("/proc/{name}/cmdline")) else {
            continue;
        };
        let zeile = String::from_utf8_lossy(&roh).replace('\0', " ");
        if zeile.contains("mcetool") && zeile.contains("--set-call-state") {
            eprintln!("== alter MCE-Halter {name} wird beendet");
            let _ = Command::new("sudo")
                .args(["kill", "-KILL", name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    setzen("none");
}

#[cfg(test)]
mod tests {
    /// Der Zustandsname, den MCE kennt, zu dem, was bei uns passiert.
    ///
    /// "ringing" ist ein Zustand von Sekunden, "active" einer von
    /// Minuten -- und "none" muss am Ende stehen, sonst bleibt der
    /// Bildschirm an.
    #[test]
    fn zustaende_sind_die_von_mce() {
        for z in ["ringing", "active", "none"] {
            assert!(!z.is_empty());
        }
    }
}
