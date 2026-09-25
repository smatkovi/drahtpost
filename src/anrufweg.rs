//! Der Weg eines Sprachanrufs: was ueber die Leitung geht.
//!
//! `anruf.rs` rechnet, dieses Modul redet. Der Ablauf ist der von
//! Telegram, und er ist so gebaut, dass keine Seite den Schluessel allein
//! bestimmt -- deshalb die Reihenfolge, die auf den ersten Blick
//! umstaendlich wirkt:
//!
//! ```text
//! Anrufer                                     Angerufener
//!   messages.getDhConfig  ->  p, g, zufall
//!   wuerfelt a, rechnet g_a
//!   phone.requestCall(sha256(g_a))  ------->  updatePhoneCall
//!                                             phoneCallRequested{g_a_hash}
//!                                             wuerfelt b, rechnet g_b
//!   updatePhoneCall        <---------------   phone.acceptCall(g_b)
//!   phoneCallAccepted{g_b}
//!   Schluessel = g_b^a
//!   phone.confirmCall(g_a, Fingerabdruck) ->  updatePhoneCall
//!                                             phoneCall{g_a_or_b = g_a}
//!                                             prueft sha256(g_a) gegen
//!                                             den Abdruck von vorhin
//!                                             Schluessel = g_a^b
//! ```
//!
//! Der Ton laeuft nicht hier durch. Steht der Schluessel, geht eine
//! Beschreibung des Gespraechs an den Tonprozess (tgcalls), und der
//! spricht von da an direkt mit der Gegenstelle.

use std::sync::Arc;

use grammers_tl_types as tl;
use serde_json::{json, Value};
use std::process::Stdio;

use tokio::io::AsyncWriteExt as _;

use crate::anruf::{self, Tausch};
use crate::anrufzustand;
use crate::telefonbruecke;
use crate::Lage;

/// Wo ein Gespraech gerade steht.
pub struct Gespraech {
    pub id: i64,
    pub zugriff: i64,
    /// Haben wir angerufen oder wurden wir angerufen? Das entscheidet
    /// spaeter auch, wie tgcalls den Schluessel herum liest.
    pub ausgehend: bool,
    pub partner: i64,
    /// Ob dieses Gespraech ein Videoanruf ist. Telegram traegt das im
    /// Kennzeichen jeder Anfrage mit; wer es weglaesst, bekommt beim
    /// Gegenueber keinen Kameraknopf zu sehen.
    pub video: bool,
    tausch: Tausch,
    /// Nur beim Anrufer: das eigene `g_a`, bis es gezeigt werden darf.
    g_a: Option<Vec<u8>>,
    /// Nur beim Angerufenen: der Abdruck, gegen den `g_a` spaeter
    /// geprueft wird. Ohne diese Pruefung koennte die Gegenseite ihr
    /// `a` nachtraeglich aussuchen.
    g_a_abdruck: Option<Vec<u8>>,
}

impl Gespraech {
    pub fn zeiger(&self) -> tl::enums::InputPhoneCall {
        tl::types::InputPhoneCall { id: self.id, access_hash: self.zugriff }.into()
    }
}

/// Unsere Protokollangaben, wie sie in jede der vier Anfragen gehoeren.
pub fn protokoll() -> tl::enums::PhoneCallProtocol {
    tl::types::PhoneCallProtocol {
        udp_p2p: true,
        udp_reflector: true,
        min_layer: anruf::MINDESTSTUFE,
        max_layer: anruf::PROTOKOLL_STUFE,
        library_versions: anruf::BIBLIOTHEKSFASSUNGEN
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
    .into()
}

/// Die Diffie-Hellman-Vorgaben des Servers holen.
///
/// `random_length: 256` heisst: gib mir 256 Bytes Zufall dazu. Die
/// verodern wir mit eigenem Zufall -- so bestimmt weder der Server noch
/// das Geraet allein den Geheimwert.
pub async fn dh_vorgaben(
    client: &grammers_client::Client,
) -> Result<(Vec<u8>, i32, Vec<u8>), String> {
    let antwort = client
        .invoke(&tl::functions::messages::GetDhConfig { version: 0, random_length: 256 })
        .await
        .map_err(|e| format!("getDhConfig: {e}"))?;
    match antwort {
        tl::enums::messages::DhConfig::Config(c) => Ok((c.p, c.g, c.random)),
        // Der Server sagt "unveraendert", wenn er meint, wir kennten die
        // Vorgaben schon. Mit version: 0 darf das nicht vorkommen --
        // dann stimmt etwas nicht, und ein Anruf auf gut Glueck waere
        // genau der falsche Umgang damit.
        tl::enums::messages::DhConfig::NotModified(_) => {
            Err("Server haelt die DH-Vorgaben fuer bekannt".into())
        }
    }
}

fn eigener_zufall() -> Vec<u8> {
    use rand::RngCore as _;
    let mut b = vec![0u8; 256];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

/// Anrufen: Schritt eins und zwei auf einmal.
pub async fn anrufen(
    client: &grammers_client::Client,
    wen: tl::enums::InputUser,
    partner: i64,
    video: bool,
) -> Result<Gespraech, String> {
    let (p, g, zufall) = dh_vorgaben(client).await?;
    let tausch = Tausch::neu(&p, g, &zufall, &eigener_zufall());
    let g_a = tausch.eigene_potenz();

    let antwort = client
        .invoke(&tl::functions::phone::RequestCall {
            video,
            user_id: wen,
            random_id: rand::random(),
            g_a_hash: anruf::potenz_abdruck(&g_a),
            protocol: protokoll(),
        })
        .await
        .map_err(|e| format!("requestCall: {e}"))?;

    let tl::enums::phone::PhoneCall::Call(c) = antwort;
    let (id, zugriff) = kennung(&c.phone_call)
        .ok_or_else(|| "requestCall lieferte kein Gespraech".to_string())?;
    Ok(Gespraech {
        id,
        zugriff,
        ausgehend: true,
        partner,
        video,
        tausch,
        g_a: Some(g_a),
        g_a_abdruck: None,
    })
}

/// Einen eingehenden Anruf merken -- angenommen ist er damit noch nicht.
pub async fn eingehend(
    client: &grammers_client::Client,
    anfrage: &tl::types::PhoneCallRequested,
) -> Result<Gespraech, String> {
    let (p, g, zufall) = dh_vorgaben(client).await?;
    let tausch = Tausch::neu(&p, g, &zufall, &eigener_zufall());
    Ok(Gespraech {
        id: anfrage.id,
        zugriff: anfrage.access_hash,
        ausgehend: false,
        partner: anfrage.admin_id,
        video: anfrage.video,
        tausch,
        g_a: None,
        g_a_abdruck: Some(anfrage.g_a_hash.clone()),
    })
}

/// Abheben: `g_b` zeigen.
pub async fn annehmen(
    client: &grammers_client::Client,
    gespraech: &Gespraech,
) -> Result<(), String> {
    client
        .invoke(&tl::functions::phone::AcceptCall {
            peer: gespraech.zeiger(),
            g_b: gespraech.tausch.eigene_potenz(),
            protocol: protokoll(),
        })
        .await
        .map_err(|e| format!("acceptCall: {e}"))?;
    Ok(())
}

/// Der Anrufer hat `g_b` bekommen: jetzt darf er `g_a` zeigen.
///
/// Gibt den Schluessel zurueck -- oder nichts, wenn `g_b` nicht brauchbar
/// war. Dann gehoert der Anruf abgebrochen, nicht gefuehrt.
pub async fn bestaetigen(
    client: &grammers_client::Client,
    gespraech: &Gespraech,
    g_b: &[u8],
) -> Result<Vec<u8>, String> {
    let g_a = gespraech.g_a.as_ref().ok_or("kein eigenes g_a")?;
    let schluessel = gespraech
        .tausch
        .schluessel(g_b)
        .ok_or("g_b liegt am Rand und ist nicht brauchbar")?;
    client
        .invoke(&tl::functions::phone::ConfirmCall {
            peer: gespraech.zeiger(),
            g_a: g_a.clone(),
            key_fingerprint: anruf::schluessel_fingerabdruck(&schluessel),
            protocol: protokoll(),
        })
        .await
        .map_err(|e| format!("confirmCall: {e}"))?;
    Ok(schluessel)
}

/// Der Angerufene sieht endlich `g_a`: pruefen und den Schluessel bilden.
///
/// Zwei Pruefungen, und beide muessen sein. Der Abdruck stellt sicher,
/// dass der Anrufer sein `a` vor unserem `b` festgelegt hat; der
/// Fingerabdruck, dass wir beide beim selben Schluessel gelandet sind.
pub fn schluessel_vom_anrufer(
    gespraech: &Gespraech,
    g_a: &[u8],
    fingerabdruck: i64,
) -> Result<Vec<u8>, String> {
    let erwartet = gespraech.g_a_abdruck.as_ref().ok_or("kein Abdruck gemerkt")?;
    if anruf::potenz_abdruck(g_a) != *erwartet {
        return Err("g_a passt nicht zu dem Abdruck von vorhin".into());
    }
    let schluessel = gespraech
        .tausch
        .schluessel(g_a)
        .ok_or("g_a liegt am Rand und ist nicht brauchbar")?;
    if anruf::schluessel_fingerabdruck(&schluessel) != fingerabdruck {
        return Err("die Fingerabdruecke der Schluessel stimmen nicht ueberein".into());
    }
    Ok(schluessel)
}

/// Auflegen.
pub async fn auflegen(
    client: &grammers_client::Client,
    gespraech: &Gespraech,
    grund: tl::enums::PhoneCallDiscardReason,
    dauer: i32,
) -> Result<(), String> {
    client
        .invoke(&tl::functions::phone::DiscardCall {
            video: gespraech.video,
            peer: gespraech.zeiger(),
            duration: dauer,
            reason: grund,
            connection_id: 0,
        })
        .await
        .map_err(|e| format!("discardCall: {e}"))?;
    Ok(())
}

/// Ein Stueck Signalisierung fuer das neue Protokoll weiterreichen.
///
/// Ab Fassung 5.0.0 handeln die beiden Tonprozesse ihre Verbindung
/// selbst aus (SDP-aehnlich, verschluesselt); Telegram ist dabei nur der
/// Briefkasten. Wir schauen in die Umschlaege nicht hinein.
pub async fn signal_senden(
    client: &grammers_client::Client,
    gespraech: &Gespraech,
    daten: Vec<u8>,
) -> Result<(), String> {
    client
        .invoke(&tl::functions::phone::SendSignalingData { peer: gespraech.zeiger(), data: daten })
        .await
        .map_err(|e| format!("sendSignalingData: {e}"))?;
    Ok(())
}

/// Kennung und Zugriffsmarke aus einer beliebigen Spielart von PhoneCall.
pub fn kennung(c: &tl::enums::PhoneCall) -> Option<(i64, i64)> {
    use tl::enums::PhoneCall as P;
    match c {
        P::Waiting(w) => Some((w.id, w.access_hash)),
        P::Requested(r) => Some((r.id, r.access_hash)),
        P::Accepted(a) => Some((a.id, a.access_hash)),
        P::Call(c) => Some((c.id, c.access_hash)),
        // Beendet und leer tragen keine Zugriffsmarke mehr -- es gibt
        // nichts mehr, wofuer man sie braeuchte.
        P::Empty(_) | P::Discarded(_) => None,
    }
}

/// Was der Tonprozess braucht, um das Gespraech zu uebernehmen.
///
/// Absichtlich JSON und absichtlich ueber einen Socket: tgcalls ist C++
/// mit WebRTC daran, und das gehoert nicht in denselben Prozess wie die
/// Signalisierung. Stuerzt der Ton ab, laeuft Telegram weiter.
pub fn uebergabe(
    gespraech: &Gespraech,
    schluessel: &[u8],
    ruf: &tl::types::PhoneCall,
) -> Result<Value, String> {
    let tl::enums::PhoneCallProtocol::Protocol(p) = &ruf.protocol;
    let fassung = anruf::fassung_waehlen(&p.library_versions)
        .ok_or("keine gemeinsame Protokollfassung mit der Gegenstelle")?;

    let mut wege = Vec::new();
    for v in &ruf.connections {
        wege.push(match v {
            tl::enums::PhoneConnection::Connection(c) => json!({
                "art": "reflektor",
                "id": c.id,
                "ip": c.ip,
                "ipv6": c.ipv6,
                "port": c.port,
                "peer_tag": hex(&c.peer_tag),
                "tcp": c.tcp,
            }),
            tl::enums::PhoneConnection::Webrtc(c) => json!({
                "art": "webrtc",
                "id": c.id,
                "ip": c.ip,
                "ipv6": c.ipv6,
                "port": c.port,
                "benutzer": c.username,
                "passwort": c.password,
                "turn": c.turn,
                "stun": c.stun,
            }),
        });
    }

    Ok(json!({
        "id": gespraech.id,
        "zugriff": gespraech.zugriff,
        "ausgehend": gespraech.ausgehend,
        "video": gespraech.video,
        "fassung": fassung,
        "schluessel": hex(schluessel),
        "p2p_erlaubt": ruf.p2p_allowed,
        "wege": wege,
    }))
}

/// Der Name, der in der Anrufansicht stehen soll.
///
/// Eine Kennung waere dort nutzlos -- man sieht sie im Sperrbildschirm und
/// weiss nicht, wer anruft. Laesst sich der Name nicht aufloesen, bleibt
/// die Kennung, denn irgendetwas muss dort stehen.
async fn anrufername(lage: &Arc<Lage>, kennung: i64) -> String {
    let Ok(gepackt) = crate::befehle::chat_oeffentlich(lage, kennung).await else {
        return kennung.to_string();
    };
    match lage.client.unpack_chat(gepackt).await {
        Ok(c) => {
            let n = c.name().trim().to_string();
            if n.is_empty() {
                kennung.to_string()
            } else {
                n
            }
        }
        Err(_) => kennung.to_string(),
    }
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gespraech_mit(p: &[u8], g: i32, ausgehend: bool) -> Gespraech {
        let tausch = Tausch::neu(p, g, &[3u8; 256], &[5u8; 256]);
        Gespraech {
            id: 1,
            zugriff: 2,
            ausgehend,
            partner: 42,
            video: false,
            tausch,
            g_a: None,
            g_a_abdruck: None,
        }
    }

    /// Ein `g_a`, das nicht zu dem vorab genannten Abdruck passt, darf
    /// nicht durchgehen. Genau davor schuetzt der Abdruck: sonst koennte
    /// der Anrufer sein `a` waehlen, NACHDEM er unser `g_b` gesehen hat.
    #[test]
    fn falsches_g_a_wird_abgelehnt() {
        let p = vec![0xffu8; 256];
        let mut g = gespraech_mit(&p, 3, false);
        let echtes = vec![7u8; 256];
        g.g_a_abdruck = Some(anruf::potenz_abdruck(&echtes));

        let gefaelscht = vec![8u8; 256];
        let fehler = schluessel_vom_anrufer(&g, &gefaelscht, 0).unwrap_err();
        assert!(fehler.contains("Abdruck"), "{fehler}");
    }

    /// Und ein Schluessel, dessen Fingerabdruck nicht zu dem genannten
    /// passt, auch nicht -- dann haben die beiden Seiten verschiedene
    /// Schluessel, und das Gespraech waere still oder abhoerbar.
    #[test]
    fn falscher_fingerabdruck_wird_abgelehnt() {
        // Eine echte Primzahl waere hier nur langsamer; geprueft wird
        // die Reihenfolge der Pruefungen, nicht die Zahlentheorie.
        let p = vec![0xffu8; 256];
        let mut g = gespraech_mit(&p, 3, false);
        let g_a = {
            let mut v = vec![0u8; 256];
            v[0] = 0x40; // gross genug fuer die Schranke
            v
        };
        g.g_a_abdruck = Some(anruf::potenz_abdruck(&g_a));

        let fehler = schluessel_vom_anrufer(&g, &g_a, 12345).unwrap_err();
        assert!(fehler.contains("Fingerabdruecke"), "{fehler}");
    }

    /// Die Kennung muss aus jeder Spielart herauskommen, die eine hat --
    /// sonst waere ein Gespraech nach dem naechsten Update nicht mehr
    /// ansprechbar.
    #[test]
    fn kennung_kommt_aus_jeder_spielart() {
        let warte: tl::enums::PhoneCall = tl::types::PhoneCallWaiting {
            video: false,
            id: 11,
            access_hash: 22,
            date: 0,
            admin_id: 1,
            participant_id: 2,
            protocol: protokoll(),
            receive_date: None,
        }
        .into();
        assert_eq!(kennung(&warte), Some((11, 22)));

        let weg: tl::enums::PhoneCall =
            tl::types::PhoneCallDiscarded { need_rating: false, need_debug: false, video: false, id: 11, reason: None, duration: None }.into();
        assert_eq!(kennung(&weg), None);
    }

    /// Hex hin und zurueck: der Rueckweg der Signalisierung geht als
    /// Text durch den Befehlssocket, und ein Byte, das dabei kippt,
    /// waere ein Gespraech, das nie zustande kommt.
    #[test]
    fn hex_hin_und_zurueck() {
        let roh: Vec<u8> = (0u8..=255).collect();
        assert_eq!(aus_hex(&hex(&roh)).unwrap(), roh);
        assert!(aus_hex("abc").is_err());
        assert!(aus_hex("zz").is_err());
    }

    /// Die Uebergabe an den Tonprozess nennt die ausgehandelte Fassung
    /// und beide Arten von Weg.
    #[test]
    fn uebergabe_nennt_fassung_und_wege() {
        let g = gespraech_mit(&[0xffu8; 256], 3, true);
        let ruf = tl::types::PhoneCall {
            p2p_allowed: true,
            video: false,
            id: 1,
            access_hash: 2,
            date: 0,
            admin_id: 1,
            participant_id: 2,
            g_a_or_b: vec![1, 2, 3],
            key_fingerprint: 7,
            protocol: tl::types::PhoneCallProtocol {
                udp_p2p: true,
                udp_reflector: true,
                min_layer: 65,
                max_layer: 92,
                library_versions: vec!["9.0.0".into(), "2.7.7".into()],
            }
            .into(),
            connections: vec![
                tl::types::PhoneConnectionWebrtc {
                    turn: true,
                    stun: false,
                    id: 5,
                    ip: "1.2.3.4".into(),
                    ipv6: String::new(),
                    port: 443,
                    username: "u".into(),
                    password: "p".into(),
                }
                .into(),
                tl::types::PhoneConnection {
                    tcp: false,
                    id: 6,
                    ip: "5.6.7.8".into(),
                    ipv6: String::new(),
                    port: 500,
                    peer_tag: vec![0xab, 0xcd],
                }
                .into(),
            ],
            start_date: 0,
            custom_parameters: None,
        };

        let v = uebergabe(&g, &[9u8; 256], &ruf).unwrap();
        // 9.0.0 ist die beste gemeinsame -- 13.0.0 koennen wir, die
        // Gegenstelle aber nicht.
        assert_eq!(v["fassung"], "9.0.0");
        assert_eq!(v["ausgehend"], true);
        assert_eq!(v["wege"][0]["art"], "webrtc");
        assert_eq!(v["wege"][1]["art"], "reflektor");
        assert_eq!(v["wege"][1]["peer_tag"], "abcd");
        assert_eq!(v["schluessel"].as_str().unwrap().len(), 512);
    }

    /// Ohne gemeinsame Fassung wird nicht telefoniert. Ein Anruf, bei
    /// dem keine Seite die Toene der anderen versteht, ist schlimmer als
    /// keiner: er klingelt und bleibt still.
    #[test]
    fn ohne_gemeinsame_fassung_keine_uebergabe() {
        let g = gespraech_mit(&[0xffu8; 256], 3, true);
        let mut ruf = tl::types::PhoneCall {
            p2p_allowed: true,
            video: false,
            id: 1,
            access_hash: 2,
            date: 0,
            admin_id: 1,
            participant_id: 2,
            g_a_or_b: vec![],
            key_fingerprint: 0,
            protocol: protokoll(),
            connections: vec![],
            start_date: 0,
            custom_parameters: None,
        };
        ruf.protocol = tl::types::PhoneCallProtocol {
            udp_p2p: true,
            udp_reflector: true,
            min_layer: 65,
            max_layer: 92,
            library_versions: vec!["4.0.0".into()],
        }
        .into();
        assert!(uebergabe(&g, &[0u8; 256], &ruf).is_err());
    }
}


// --- Der Draht zur Oberflaeche und zum Ton ----------------------------

/// Wo der Tonprozess horcht.
///
/// Laeuft er nicht, wird das Gespraech trotzdem aufgebaut und beendet --
/// es ist dann nur still. Das ist besser, als die Signalisierung an einem
/// fehlenden Socket scheitern zu lassen: die Gegenstelle bekommt so
/// wenigstens ein ordentliches Auflegen und nicht eine Leitung, die
/// klingelt und nie antwortet.
pub fn ton_socket() -> std::path::PathBuf {
    crate::datenverzeichnis().join("ton.sock")
}

/// Wo der Tonprozess liegt.
pub const TONPROGRAMM: &str = "/opt/drahtpost/tonprozess";

async fn an_den_ton(was: &Value) -> bool {
    let pfad = ton_socket();
    for versuch in 0..2 {
        match tokio::net::UnixStream::connect(&pfad).await {
            Ok(mut strom) => {
                let zeile = format!("{was}\n");
                if let Err(e) = strom.write_all(zeile.as_bytes()).await {
                    eprintln!("⚠ Tonprozess: {e}");
                    return false;
                }
                return true;
            }
            // Beim ersten Fehlschlag den Tonprozess starten -- wie die
            // Nachrichtenbruecke es mit uns macht. Er laeuft nicht die
            // ganze Zeit mit: 15 MB WebRTC im Speicher zu halten, waehrend
            // niemand telefoniert, waere auf einem Geraet mit 1 GB nicht
            // zu bezahlen.
            Err(_) if versuch == 0 && std::path::Path::new(TONPROGRAMM).exists() => {
                let _ = std::fs::remove_file(&pfad);
                eprintln!("== starte Tonprozess");
                // std statt tokio::process: "process" waere ein
                // Merkmal, das wir uns sonst nirgends erkaufen muessten.
                // Seine Ausgabe gehoert in eine Datei, nicht nach
                // /dev/null. Beim ersten echten Anruf stuerzte er ab,
                // und der Absturzmelder schrieb sein "Signal 11 bei
                // pc=..." ins Nichts -- uebrig blieben zwei Zombies und
                // keine Erklaerung.
                let protokoll = crate::datenverzeichnis().join("ton.log");
                let hin = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&protokoll);
                let (aus, fehler) = match hin {
                    Ok(f) => match f.try_clone() {
                        Ok(g) => (Stdio::from(f), Stdio::from(g)),
                        Err(_) => (Stdio::null(), Stdio::null()),
                    },
                    Err(_) => (Stdio::null(), Stdio::null()),
                };
                match std::process::Command::new(TONPROGRAMM)
                    .stdout(aus)
                    .stderr(fehler)
                    .spawn()
                {
                    Ok(mut kind) => {
                        // Einsammeln, sonst bleibt bei jedem Absturz ein
                        // Zombie stehen -- nach drei Anrufen standen
                        // zwei davon in der Prozessliste.
                        std::thread::spawn(move || {
                            match kind.wait() {
                                Ok(stand) if !stand.success() => {
                                    eprintln!("⚠ Tonprozess endete: {stand}");
                                }
                                _ => {}
                            }
                        });
                        // Er braucht einen Augenblick, bis der Socket
                        // steht. Warten ist hier richtig: die Alternative
                        // waere, den ersten Anruf stumm zu lassen.
                        for _ in 0..40 {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            if pfad.exists() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠ Tonprozess startet nicht: {e}");
                        return false;
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠ Tonprozess nicht erreichbar ({}): {e}", pfad.display());
                return false;
            }
        }
    }
    false
}

/// Was der Tonprozess ueber die Leitung meldet, an die Oberflaeche.
pub async fn zustand_melden(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let zustand = args.get("state").and_then(|x| x.as_str()).unwrap_or("");
    let id = args.get("call_id").and_then(|x| x.as_i64()).unwrap_or(0);
    eprintln!("== Leitung: {zustand}");
    lage.melden(json!({
        "event": "call_state",
        "data": {"call_id": id, "state": zustand},
    }));
    Ok(json!({"ok": true}))
}

/// Laeuft gerade ein Gespraech?
///
/// Die Frage stellt nicht nur die Oberflaeche, sondern auch das
/// Installationsskript: einen Austausch mitten im Anruf hat es schon
/// gegeben, und er legt ihn.
pub async fn stand(lage: &Arc<Lage>) -> Result<Value, String> {
    let halter = lage.gespraech.lock().await;
    Ok(match halter.as_ref() {
        Some(g) => json!({
            "active": true,
            "call_id": g.id,
            "outgoing": g.ausgehend,
            "video": g.video,
            "peer": g.partner,
        }),
        None => json!({"active": false}),
    })
}

/// Laesst sich der Ton ueberhaupt erreichen?
///
/// Ohne diesen Befehl zeigt sich der Weg vom Daemon zum Tonprozess erst
/// beim ersten echten Anruf -- und ein Fehler dort ist ein Gespraech, das
/// zustande kommt und still bleibt. Genau die Sorte Fehler, die man nicht
/// erst am Telefon bemerken will.
pub async fn tonprobe(lage: &Arc<Lage>) -> Result<Value, String> {
    let _ = lage;
    let da = an_den_ton(&json!({"befehl": "probe"})).await;
    if da {
        Ok(json!({"ok": true, "socket": ton_socket().to_string_lossy()}))
    } else {
        Err(format!("Tonprozess nicht erreichbar ({})", ton_socket().display()))
    }
}

/// Die Kamera im laufenden Gespraech an- oder abschalten.
///
/// Telegram nennt das "Call Upgrade": aus einem Sprachanruf wird einer
/// mit Bild, ohne ihn neu aufzubauen. Die Signalisierung dafuer macht
/// tgcalls selbst -- wir sagen nur dem Tonprozess Bescheid.
pub async fn kamera(lage: &Arc<Lage>, an: bool) -> Result<Value, String> {
    if lage.gespraech.lock().await.is_none() {
        return Err("kein Gespraech".into());
    }
    if an_den_ton(&json!({"befehl": "kamera", "an": an})).await {
        Ok(json!({"ok": true}))
    } else {
        Err("Tonprozess nicht erreichbar".into())
    }
}

/// Das Mikrofon im laufenden Gespraech abschalten.
pub async fn stummschalten(lage: &Arc<Lage>, an: bool) -> Result<Value, String> {
    if lage.gespraech.lock().await.is_none() {
        return Err("kein Gespraech".into());
    }
    if an_den_ton(&json!({"befehl": "stumm", "an": an})).await {
        Ok(json!({"ok": true}))
    } else {
        Err("Tonprozess nicht erreichbar".into())
    }
}

/// Ein Anruf hinaus.
pub async fn starten(lage: &Arc<Lage>, kennung: i64, video: bool) -> Result<Value, String> {
    if lage.gespraech.lock().await.is_some() {
        return Err("es laeuft schon ein Gespraech".into());
    }
    eprintln!("== Anruf hinaus an {kennung} (Video: {video})");
    let chat = crate::befehle::chat_oeffentlich(lage, kennung).await?;
    let g = anrufen(&lage.client, chat.to_input_user_lossy(), kennung, video).await?;
    eprintln!("== requestCall angenommen, Gespraech {}", g.id);
    anrufzustand::setzen("ringing");
    let antwort = json!({"ok": true, "call_id": g.id});
    *lage.gespraech.lock().await = Some(g);
    Ok(antwort)
}

/// Abheben.
pub async fn abheben(lage: &Arc<Lage>) -> Result<Value, String> {
    // Abgehoben werden kann an zwei Stellen: in der Anrufansicht des
    // Telefons und in der App. Geschieht es in der App, waehrend das
    // Telefon noch klingelt, muss dort Schluss sein -- sonst klingelt es
    // weiter, waehrend das Gespraech laengst laeuft, und der Ton ginge
    // an eine Seite, die nie abgehoben hat.
    //
    // Kommt der Aufruf von der Bruecke selbst, steht das Gespraech dort
    // schon; dann ist hier nichts zu tun.
    if telefonbruecke::telefon_da() && !telefonbruecke::im_gespraech() {
        eprintln!("== in der App abgehoben -- das Telefon hoert auf zu klingeln");
        telefonbruecke::auflegen("anderswo-angenommen");
        anrufzustand::setzen("active");
    }
    let gespraech = lage.gespraech.lock().await;
    let g = gespraech.as_ref().ok_or("kein Gespraech zum Abheben")?;
    if g.ausgehend {
        return Err("ein eigener Anruf wird nicht abgehoben".into());
    }
    annehmen(&lage.client, g).await?;
    Ok(json!({"ok": true}))
}

/// Auflegen -- und zwar auch dann, wenn der Server das Gespraech schon
/// vergessen hat. Was hier haengenbleibt, blockiert das naechste.
pub async fn beenden(lage: &Arc<Lage>, grund: &str) -> Result<Value, String> {
    let mut halter = lage.gespraech.lock().await;
    let g = halter.take().ok_or("kein Gespraech zum Auflegen")?;
    let r: tl::enums::PhoneCallDiscardReason = match grund {
        "busy" => tl::types::PhoneCallDiscardReasonBusy {}.into(),
        "missed" => tl::types::PhoneCallDiscardReasonMissed {}.into(),
        "disconnect" => tl::types::PhoneCallDiscardReasonDisconnect {}.into(),
        _ => tl::types::PhoneCallDiscardReasonHangup {}.into(),
    };
    // Den MCE-Anrufzustand nur zuruecknehmen, wenn wir ihn auch halten.
    // Fuehrt die Telefon-App das Gespraech, gehoert er ihr -- und ein
    // "keine Anrufe" von uns kaeme mitten in ihren Abbau hinein.
    if !telefonbruecke::im_gespraech() {
        anrufzustand::setzen("none");
    }
    telefonbruecke::auflegen("beendet");
    let ergebnis = auflegen(&lage.client, &g, r, 0).await;
    an_den_ton(&json!({"befehl": "auflegen", "id": g.id})).await;
    ergebnis?;
    Ok(json!({"ok": true}))
}

/// Ein `updatePhoneCall` in Ereigniszeilen uebersetzen -- und dabei den
/// naechsten Schritt des Schluesseltauschs tun.
pub async fn update(lage: &Arc<Lage>, ruf: &tl::enums::PhoneCall) -> Vec<Value> {
    use tl::enums::PhoneCall as P;
    match ruf {
        // Wir haben angerufen, der Server hat es angenommen.
        P::Waiting(w) => vec![json!({
            "event": "call_state",
            "data": {"call_id": w.id, "state": "waiting"},
        })],

        // Jemand ruft uns an.
        P::Requested(r) => {
            eprintln!("== Anruf herein von {} (Video: {})", r.admin_id, r.video);
            if lage.gespraech.lock().await.is_some() {
                // Besetzt. Ohne diese Antwort klingelt es bei der
                // Gegenstelle, bis sie von selbst aufgibt.
                let zeiger: tl::enums::InputPhoneCall =
                    tl::types::InputPhoneCall { id: r.id, access_hash: r.access_hash }.into();
                let _ = lage
                    .client
                    .invoke(&tl::functions::phone::DiscardCall {
                        video: false,
                        peer: zeiger,
                        duration: 0,
                        reason: tl::types::PhoneCallDiscardReasonBusy {}.into(),
                        connection_id: 0,
                    })
                    .await;
                return vec![];
            }
            match eingehend(&lage.client, r).await {
                Ok(g) => {
                    let id = g.id;
                    let von = g.partner;
                    *lage.gespraech.lock().await = Some(g);
                    // Das Telefon klingeln lassen -- in seiner eigenen
                    // Anrufansicht. Angenommen wird erst, wenn es meldet,
                    // dass abgehoben wurde; wer den Telegram-Anruf schon
                    // hier annaehme, liesse die Gegenstelle ins Leere
                    // reden, bis jemand das Telefon erreicht.
                    let name = anrufername(lage, von).await;
                    if telefonbruecke::klingeln(&name) {
                        // Der Anrufzustand gehoert dann der Telefonie des
                        // Systems. Ihn daneben selbst zu halten hiesse,
                        // zwei Anrufe zu fuehren -- und der Klingelton
                        // kaeme zweimal.
                    } else {
                        anrufzustand::setzen("ringing");
                    }
                    vec![json!({
                        "event": "call_incoming",
                        "data": {"call_id": id, "from": von, "video": r.video},
                    })]
                }
                Err(e) => vec![fehlerzeile(r.id, &e)],
            }
        }

        // Die Gegenstelle hat abgehoben und zeigt ihr g_b.
        P::Accepted(a) => {
            eprintln!("== Gegenstelle hat abgehoben, g_b ist da");
            let halter = lage.gespraech.lock().await;
            let Some(g) = halter.as_ref() else {
                return vec![];
            };
            match bestaetigen(&lage.client, g, &a.g_b).await {
                Ok(_) => vec![json!({
                    "event": "call_state",
                    "data": {"call_id": a.id, "state": "accepted"},
                })],
                Err(e) => vec![fehlerzeile(a.id, &e)],
            }
        }

        // Der Schluessel steht: ab hier redet der Ton.
        P::Call(c) => {
            eprintln!("== Schluessel steht, {} Wege", c.connections.len());
            let halter = lage.gespraech.lock().await;
            let Some(g) = halter.as_ref() else {
                return vec![];
            };
            // Der Anrufer kennt den Schluessel schon aus Schritt drei;
            // der Angerufene bildet ihn jetzt -- und prueft dabei beides.
            let schluessel = if g.ausgehend {
                g.tausch.schluessel(&c.g_a_or_b).ok_or_else(|| "g_b unbrauchbar".to_string())
            } else {
                schluessel_vom_anrufer(g, &c.g_a_or_b, c.key_fingerprint)
            };
            let schluessel = match schluessel {
                Ok(s) => s,
                Err(e) => return vec![fehlerzeile(c.id, &e)],
            };
            let beschreibung = match uebergabe(g, &schluessel, c) {
                Ok(v) => v,
                Err(e) => return vec![fehlerzeile(c.id, &e)],
            };
            let zeichen = anruf::pruefzeichen_stellen(&schluessel, &c.g_a_or_b, 333);
            // Wohin der Ton geht, haengt daran, ob die Telefon-App das
            // Gespraech fuehrt. Tut sie es, holt sich der Tonprozess
            // seinen Takt von ihrem RTP; tut sie es nicht, wartete er
            // auf Rahmen, die nie kaemen -- also dann PulseAudio.
            let mut beschreibung = beschreibung;
            if telefonbruecke::im_gespraech() {
                beschreibung["ton"] = json!("sip");
            } else {
                beschreibung["ton"] = json!("puls");
                anrufzustand::setzen("active");
            }
            an_den_ton(&json!({"befehl": "anrufen", "gespraech": beschreibung})).await;
            vec![json!({
                "event": "call_ready",
                "data": {
                    "call_id": c.id,
                    "version": beschreibung["fassung"],
                    "emoji_indices": zeichen,
                },
            })]
        }

        P::Discarded(d) => {
            eprintln!("== Gespraech {} beendet", d.id);
            let mut halter = lage.gespraech.lock().await;
            if halter.as_ref().map(|g| g.id) == Some(d.id) {
                halter.take();
            }
            if !telefonbruecke::im_gespraech() {
                anrufzustand::setzen("none");
            }
            telefonbruecke::auflegen("beendet");
            an_den_ton(&json!({"befehl": "auflegen", "id": d.id})).await;
            vec![json!({
                "event": "call_ended",
                "data": {"call_id": d.id, "duration": d.duration.unwrap_or(0)},
            })]
        }

        P::Empty(_) => vec![],
    }
}

/// Ein Stueck Signalisierung fuer den Tonprozess.
pub async fn signal_herein(lage: &Arc<Lage>, gespraech_id: i64, daten: &[u8]) {
    if lage.gespraech.lock().await.as_ref().map(|g| g.id) != Some(gespraech_id) {
        return;
    }
    an_den_ton(&json!({
        "befehl": "signal",
        "id": gespraech_id,
        "daten": hex(daten),
    }))
    .await;
}

/// Der Rueckweg: was der Tonprozess der Gegenstelle sagen will.
///
/// Er kommt ueber denselben Befehlssocket herein wie alles andere --
/// dann braucht es keinen zweiten Draht, und der Tonprozess spricht mit
/// der Drahtpost genau eine Sprache.
pub async fn signal_hinaus(lage: &Arc<Lage>, daten_hex: &str) -> Result<Value, String> {
    let halter = lage.gespraech.lock().await;
    let g = halter.as_ref().ok_or("kein Gespraech")?;
    let daten = aus_hex(daten_hex)?;
    signal_senden(&lage.client, g, daten).await?;
    Ok(json!({"ok": true}))
}

fn aus_hex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("ungerade Laenge".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn fehlerzeile(id: i64, grund: &str) -> Value {
    eprintln!("⚠ Anruf {id}: {grund}");
    json!({"event": "call_failed", "data": {"call_id": id, "reason": grund}})
}
