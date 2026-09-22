//! Kennung -> Chat.
//!
//! Die Oberflaeche und die Bruecke schicken Chat-Kennungen so, wie
//! Telethon sie schreibt: eine Zahl, bei Gruppen negativ, bei Kanaelen
//! mit -100 davor. MTProto selbst kann damit nichts anfangen -- es
//! braucht zu jeder Kennung einen access_hash, den der Server beim
//! ersten Sehen mitgibt und der danach nie wieder kommt.
//!
//! grammers packt beides in einen PackedChat, der sich als Hex-Zeichen-
//! kette speichern laesst. Genau das tut diese Datei: sie merkt sich
//! jeden Chat, der ihr unterkommt, und schreibt die Tabelle neben die
//! Sitzung. Ohne sie waere nach jedem Neustart erst ein voller
//! Dialogdurchlauf noetig, bevor sich ueberhaupt etwas senden laesst --
//! ueber GPRS dauert das Minuten.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use grammers_session::{PackedChat, PackedType};

/// Telethons Schreibweise: Benutzer positiv, kleine Gruppen negativ,
/// Kanaele und Supergruppen mit der -100-Vorsilbe.
pub fn markiert(p: &PackedChat) -> i64 {
    match p.ty {
        PackedType::User | PackedType::Bot => p.id,
        PackedType::Chat => -p.id,
        _ => -1_000_000_000_000 - p.id,
    }
}

pub struct Verzeichnis {
    tabelle: Mutex<HashMap<i64, String>>,
    pfad: PathBuf,
    /// Gespeichert wird nur, wenn sich etwas geaendert hat.
    schmutzig: Mutex<bool>,
}

impl Verzeichnis {
    pub fn laden(daten: &Path) -> Self {
        let pfad = daten.join("verzeichnis.json");
        let tabelle = std::fs::read_to_string(&pfad)
            .ok()
            .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| k.parse::<i64>().ok().map(|k| (k, v)))
                    .collect::<HashMap<i64, String>>()
            })
            .unwrap_or_default();
        eprintln!("== Verzeichnis: {} Chats", tabelle.len());
        Self {
            tabelle: Mutex::new(tabelle),
            pfad,
            schmutzig: Mutex::new(false),
        }
    }

    pub fn leer(&self) -> bool {
        self.tabelle.lock().unwrap().is_empty()
    }

    pub fn merken(&self, p: &PackedChat) {
        let k = markiert(p);
        let hex = p.to_hex();
        let mut t = self.tabelle.lock().unwrap();
        if t.get(&k).map(|a| a == &hex).unwrap_or(false) {
            return;
        }
        t.insert(k, hex);
        *self.schmutzig.lock().unwrap() = true;
    }

    pub fn finden(&self, kennung: i64) -> Option<PackedChat> {
        let t = self.tabelle.lock().unwrap();
        t.get(&kennung).and_then(|h| PackedChat::from_hex(h).ok())
    }

    pub fn sichern(&self) {
        {
            let mut s = self.schmutzig.lock().unwrap();
            if !*s {
                return;
            }
            *s = false;
        }
        let t = self.tabelle.lock().unwrap();
        let text: HashMap<String, &String> =
            t.iter().map(|(k, v)| (k.to_string(), v)).collect();
        if let Ok(j) = serde_json::to_string(&text) {
            let vorlaeufig = self.pfad.with_extension("neu");
            if std::fs::write(&vorlaeufig, j).is_ok() {
                let _ = std::fs::rename(&vorlaeufig, &self.pfad);
            }
        }
    }
}
