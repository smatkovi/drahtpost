//! Welche Chats stummgeschaltet sind -- und seit wann bis wann.
//!
//! Die Stummschaltung ist in Telegram eine Eigenschaft des Dialogs, nicht
//! des Chats: sie steckt in `notify_settings` und besteht aus einem
//! Zeitpunkt, bis zu dem geschwiegen wird (`mute_until`; fuer "auf immer"
//! schickt der Server 2147483647). Deshalb wird hier der Zeitpunkt
//! gemerkt und nicht ein Ja/Nein -- "acht Stunden stumm" muss von selbst
//! ablaufen, auch wenn der Daemon zwischendurch nie wieder Dialoge holt.
//!
//! Warum die Tabelle auf der Platte liegt: mit `catch_up` kommt gleich
//! nach dem Verbinden alles herein, was ueber Nacht angefallen ist --
//! lange bevor der erste Dialogdurchlauf fertig ist. Ohne gespeicherte
//! Tabelle waere also nach jedem Einschalten genau das in der
//! Nachrichten-App, was der Benutzer stummgeschaltet hat.
//!
//! `silent` aus denselben Einstellungen wird hier bewusst nicht
//! ausgewertet: das heisst "ohne Ton benachrichtigen", nicht "gar nicht
//! benachrichtigen". Die Dialogliste zeigt es weiterhin an (formen.rs),
//! unterdrueckt wird nur nach `mute_until`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use grammers_tl_types as tl;

pub struct Stummliste {
    /// markierte Kennung -> Zeitpunkt, bis zu dem stumm.
    tabelle: Mutex<HashMap<i64, i32>>,
    pfad: PathBuf,
    /// Lag beim Start schon eine Tabelle da? Wenn nicht, muessen die
    /// Dialoge einmal geholt werden -- auch dann, wenn das Verzeichnis
    /// voll ist. Genau der Fall bei einem Drahtpost, das ueber eine
    /// aeltere Fassung installiert wird.
    bekannt: bool,
    schmutzig: Mutex<bool>,
}

impl Stummliste {
    pub fn laden(daten: &Path) -> Self {
        let pfad = daten.join("stumm.json");
        let roh = std::fs::read_to_string(&pfad).ok();
        let bekannt = roh.is_some();
        let tabelle = roh
            .and_then(|t| serde_json::from_str::<HashMap<String, i32>>(&t).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| k.parse::<i64>().ok().map(|k| (k, v)))
                    .collect::<HashMap<i64, i32>>()
            })
            .unwrap_or_default();
        eprintln!("== stumm: {} Chats", tabelle.len());
        Self {
            tabelle: Mutex::new(tabelle),
            pfad,
            bekannt,
            schmutzig: Mutex::new(false),
        }
    }

    /// Ist die Tabelle ueberhaupt schon einmal gefuellt worden? Eine leere
    /// Datei ist eine gueltige Antwort ("nichts ist stumm") -- das
    /// Fehlen der Datei ist es nicht.
    pub fn bekannt(&self) -> bool {
        self.bekannt
    }

    pub fn setzen(&self, kennung: i64, bis: i32) {
        let mut t = self.tabelle.lock().unwrap();
        // Nur das Stumme wird aufgehoben; alles andere ist der Normalfall
        // und braucht keinen Eintrag.
        let alt = t.get(&kennung).copied();
        let neu = if bis > jetzt() { Some(bis) } else { None };
        if alt == neu {
            return;
        }
        match neu {
            Some(b) => t.insert(kennung, b),
            None => t.remove(&kennung),
        };
        *self.schmutzig.lock().unwrap() = true;
    }

    pub fn stumm(&self, kennung: i64) -> bool {
        self.tabelle
            .lock()
            .unwrap()
            .get(&kennung)
            .map(|bis| *bis > jetzt())
            .unwrap_or(false)
    }

    pub fn sichern(&self) {
        {
            let mut s = self.schmutzig.lock().unwrap();
            if !*s {
                return;
            }
            *s = false;
        }
        let jetzt = jetzt();
        let t = self.tabelle.lock().unwrap();
        let text: HashMap<String, i32> = t
            .iter()
            .filter(|(_, bis)| **bis > jetzt)
            .map(|(k, bis)| (k.to_string(), *bis))
            .collect();
        if let Ok(j) = serde_json::to_string(&text) {
            let vorlaeufig = self.pfad.with_extension("neu");
            if std::fs::write(&vorlaeufig, j).is_ok() {
                let _ = std::fs::rename(&vorlaeufig, &self.pfad);
            }
        }
    }
}

fn jetzt() -> i32 {
    chrono::Utc::now().timestamp() as i32
}

/// Der Zeitpunkt aus den Benachrichtigungseinstellungen. 0 heisst: nicht
/// stumm.
pub fn bis_aus_einstellungen(n: &tl::enums::PeerNotifySettings) -> i32 {
    let tl::enums::PeerNotifySettings::Settings(n) = n;
    n.mute_until.unwrap_or(0)
}

/// Aus dem rohen Dialog, so wie er aus iter_dialogs faellt.
pub fn bis_aus_dialog(roh: &tl::enums::Dialog) -> i32 {
    match roh {
        tl::enums::Dialog::Dialog(d) => bis_aus_einstellungen(&d.notify_settings),
        tl::enums::Dialog::Folder(_) => 0,
    }
}

/// Telethons markierte Kennung aus einem rohen Peer -- dieselbe Rechnung
/// wie in verzeichnis::markiert, nur ohne PackedChat. updateNotifySettings
/// traegt keinen Chat mit sich, nur den Peer.
pub fn kennung_aus_peer(p: &tl::enums::Peer) -> i64 {
    match p {
        tl::enums::Peer::User(u) => u.user_id,
        tl::enums::Peer::Chat(c) => -c.chat_id,
        tl::enums::Peer::Channel(c) => -1_000_000_000_000 - c.channel_id,
    }
}

#[cfg(test)]
mod proben {
    use super::*;

    fn liste(pfad: &Path) -> Stummliste {
        Stummliste::laden(pfad)
    }

    #[test]
    fn abgelaufene_stummschaltung_gilt_nicht_mehr() {
        let d = std::env::temp_dir().join("stummprobe1");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let l = liste(&d);
        l.setzen(-100123, jetzt() + 3600);
        l.setzen(-100456, jetzt() - 1);
        assert!(l.stumm(-100123));
        assert!(!l.stumm(-100456), "abgelaufenes mute_until ist keine Stummschaltung");
        assert!(!l.stumm(-100789), "unbekannte Chats sind nicht stumm");
    }

    #[test]
    fn tabelle_ueberlebt_den_neustart() {
        let d = std::env::temp_dir().join("stummprobe2");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        {
            let l = liste(&d);
            assert!(!l.bekannt(), "ohne Datei ist nichts bekannt");
            l.setzen(-1001461414594, i32::MAX);
            l.sichern();
        }
        let l = liste(&d);
        assert!(l.bekannt(), "mit Datei gilt die Tabelle als gefuellt");
        assert!(l.stumm(-1001461414594));

        // Lautstellen loescht den Eintrag, und zwar auch auf der Platte.
        l.setzen(-1001461414594, 0);
        l.sichern();
        assert!(!liste(&d).stumm(-1001461414594));
    }

    #[test]
    fn peer_wird_zur_markierten_kennung() {
        use tl::enums::Peer;
        assert_eq!(
            kennung_aus_peer(&Peer::User(tl::types::PeerUser { user_id: 93061901 })),
            93061901
        );
        assert_eq!(
            kennung_aus_peer(&Peer::Chat(tl::types::PeerChat { chat_id: 4711 })),
            -4711
        );
        assert_eq!(
            kennung_aus_peer(&Peer::Channel(tl::types::PeerChannel {
                channel_id: 1461414594
            })),
            -1001461414594
        );
    }
}
