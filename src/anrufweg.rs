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

use crate::anruf::{self, Tausch};

/// Wo ein Gespraech gerade steht.
pub struct Gespraech {
    pub id: i64,
    pub zugriff: i64,
    /// Haben wir angerufen oder wurden wir angerufen? Das entscheidet
    /// spaeter auch, wie tgcalls den Schluessel herum liest.
    pub ausgehend: bool,
    pub partner: i64,
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
) -> Result<Gespraech, String> {
    let (p, g, zufall) = dh_vorgaben(client).await?;
    let tausch = Tausch::neu(&p, g, &zufall, &eigener_zufall());
    let g_a = tausch.eigene_potenz();

    let antwort = client
        .invoke(&tl::functions::phone::RequestCall {
            video: false,
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
            video: false,
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
        "fassung": fassung,
        "schluessel": hex(schluessel),
        "p2p_erlaubt": ruf.p2p_allowed,
        "wege": wege,
    }))
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
