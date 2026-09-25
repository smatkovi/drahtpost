//! Sprachanrufe: der Schluesseltausch.
//!
//! Telegram baut den Schluessel eines Gespraechs mit Diffie-Hellman auf,
//! und zwar so, dass keine der beiden Seiten ihn allein bestimmen kann:
//!
//! 1. Der Anrufer wuerfelt `a`, rechnet `g_a = g^a mod p` und schickt nur
//!    **den SHA256-Abdruck davon** mit `phone.requestCall`. Das ist der
//!    entscheidende Kniff: haette er `g_a` selbst geschickt, koennte die
//!    Gegenseite ihr `b` danach aussuchen und den Schluessel steuern.
//! 2. Der Angerufene wuerfelt `b` und antwortet mit `g_b`.
//! 3. Erst jetzt zeigt der Anrufer sein `g_a` und bestaetigt mit dem
//!    Fingerabdruck des Schluessels. Der Angerufene prueft, dass der
//!    Abdruck aus Schritt 1 dazu passt.
//!
//! Beide rechnen denselben Schluessel aus (`g_b^a` bzw. `g_a^b`), und aus
//! ihm und `g_a` entstehen vier Pruefzeichen, die beide Seiten einander
//! vorlesen koennen. Stimmen sie ueberein, sitzt niemand dazwischen.
//!
//! Hier steht nur die Rechnung. Was ueber die Leitung geht, macht
//! `befehle.rs`; den Ton macht libtgvoip in einem eigenen Prozess.

use num_bigint::BigUint;
use num_traits::One;
use sha1::Digest as _;

/// Die Protokollangaben, die wir dem Server nennen.
///
/// Stufe 92 ist, was unser libtgvoip 2.5 spricht (`GetConnectionMaxLayer`
/// gibt sie aus). Die Versionsliste nennt zusaetzlich 2.4.4, die
/// verbreitete Fassung der libtgvoip-Zeit -- ein Server, der nur die
/// kennt, findet so trotzdem etwas Bekanntes.
pub const PROTOKOLL_STUFE: i32 = 92;
pub const BIBLIOTHEKSFASSUNGEN: [&str; 2] = ["2.4.4", "2.5"];

/// Der kleinste erlaubte Abstand zu den Raendern.
///
/// Telegram verlangt, dass `g_a` und `g_b` weder nahe bei 1 noch nahe bei
/// `p-1` liegen: `2^(2048-64) <= x <= p - 2^(2048-64)`. Sonst waere der
/// Schluesselraum klein genug, um ihn abzusuchen.
fn untere_schranke() -> BigUint {
    BigUint::one() << (2048u32 - 64)
}

/// Prueft eine empfangene Potenz, bevor daraus ein Schluessel wird.
pub fn potenz_ist_brauchbar(p: &BigUint, x: &BigUint) -> bool {
    let unten = untere_schranke();
    if x <= &BigUint::one() {
        return false;
    }
    let oben = p - &unten;
    x >= &unten && x <= &oben && x < p
}

/// Eine halbe Seite des Tauschs.
pub struct Tausch {
    p: BigUint,
    g: BigUint,
    /// Der eigene Geheimwert. Verlaesst dieses Programm nie.
    geheim: BigUint,
}

impl Tausch {
    /// `zufall` sind die 256 Bytes, die der Server in `messages.dhConfig`
    /// mitliefert, mit eigenem Zufall verodert -- so bestimmt weder
    /// Server noch Geraet allein den Geheimwert.
    pub fn neu(p: &[u8], g: i32, zufall: &[u8], eigener_zufall: &[u8]) -> Self {
        let mut roh = zufall.to_vec();
        for (i, b) in eigener_zufall.iter().enumerate() {
            if i < roh.len() {
                roh[i] ^= b;
            }
        }
        Tausch {
            p: BigUint::from_bytes_be(p),
            g: BigUint::from(g as u32),
            geheim: BigUint::from_bytes_be(&roh),
        }
    }

    /// `g^geheim mod p`, als 256 Bytes mit fuehrenden Nullen.
    pub fn eigene_potenz(&self) -> Vec<u8> {
        auf_256(&self.g.modpow(&self.geheim, &self.p))
    }

    /// Der gemeinsame Schluessel aus der Potenz der Gegenseite.
    ///
    /// Gibt nichts zurueck, wenn die Potenz nicht brauchbar ist -- dann
    /// gehoert der Anruf abgebrochen, nicht gefuehrt.
    pub fn schluessel(&self, fremde_potenz: &[u8]) -> Option<Vec<u8>> {
        let x = BigUint::from_bytes_be(fremde_potenz);
        if !potenz_ist_brauchbar(&self.p, &x) {
            return None;
        }
        Some(auf_256(&x.modpow(&self.geheim, &self.p)))
    }
}

/// Auf 256 Bytes bringen: fuehrende Nullen gehoeren dazu, sonst stimmt
/// spaeter kein Abdruck mehr.
fn auf_256(z: &BigUint) -> Vec<u8> {
    let mut b = z.to_bytes_be();
    while b.len() < 256 {
        b.insert(0, 0);
    }
    b
}

/// Der Abdruck von `g_a`, den der Anrufer vorab schickt.
pub fn potenz_abdruck(g_a: &[u8]) -> Vec<u8> {
    sha2::Sha256::digest(g_a).to_vec()
}

/// Der Fingerabdruck des Schluessels: die letzten acht Bytes seines
/// SHA1, als vorzeichenbehaftete Zahl gelesen.
pub fn schluessel_fingerabdruck(schluessel: &[u8]) -> i64 {
    let h = sha1::Sha1::digest(schluessel);
    let mut acht = [0u8; 8];
    acht.copy_from_slice(&h[12..20]);
    i64::from_le_bytes(acht)
}

/// Die vier Pruefzeichen, die beide Seiten einander vorlesen.
///
/// Gerechnet wird SHA256 ueber Schluessel und `g_a`; die 32 Bytes
/// zerfallen in vier Achtergruppen, jede wird als Zahl gelesen und auf
/// die Zeichenliste abgebildet. Beide Seiten kommen auf dieselben vier --
/// wenn nicht, sitzt jemand dazwischen.
pub fn pruefzeichen_stellen(schluessel: &[u8], g_a: &[u8], anzahl_zeichen: u64) -> [u64; 4] {
    let mut h = sha2::Sha256::new();
    h.update(schluessel);
    h.update(g_a);
    let d = h.finalize();
    let mut stellen = [0u64; 4];
    for (i, stelle) in stellen.iter_mut().enumerate() {
        let mut acht = [0u8; 8];
        acht.copy_from_slice(&d[i * 8..(i + 1) * 8]);
        // Grosse Endianzahl, und das oberste Bit weg: Telegram liest sie
        // als vorzeichenlos, aber nur 63 Bit breit.
        let wert = u64::from_be_bytes(acht) & 0x7fff_ffff_ffff_ffff;
        *stelle = wert % anzahl_zeichen;
    }
    stellen
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Beide Seiten muessen auf denselben Schluessel kommen. Gerechnet
    /// wird hier mit einer kleinen Primzahl, damit der Test schnell ist;
    /// die Rechnung ist dieselbe.
    #[test]
    fn beide_seiten_kommen_auf_denselben_schluessel() {
        // 2^256 - 189 ist prim, und gross genug, dass die Zahlen nicht
        // zufaellig gleich werden.
        let p: BigUint = (BigUint::one() << 256u32) - BigUint::from(189u32);

        let anrufer = Tausch {
            p: p.clone(),
            g: BigUint::from(3u32),
            geheim: BigUint::from(123456789u64),
        };
        let angerufener = Tausch {
            p: p.clone(),
            g: BigUint::from(3u32),
            geheim: BigUint::from(987654321u64),
        };

        let g_a = anrufer.eigene_potenz();
        let g_b = angerufener.eigene_potenz();

        // Die Schranken gelten fuer 2048 Bit; hier nur die Rechnung.
        let schluessel_a = auf_256(&BigUint::from_bytes_be(&g_b).modpow(&anrufer.geheim, &p));
        let schluessel_b = auf_256(&BigUint::from_bytes_be(&g_a).modpow(&angerufener.geheim, &p));
        assert_eq!(schluessel_a, schluessel_b);
    }

    /// Eine Potenz nahe am Rand ist nicht brauchbar -- sonst liesse sich
    /// der Schluesselraum absuchen.
    #[test]
    fn raender_werden_abgelehnt() {
        let p: BigUint = (BigUint::one() << 2048u32) - BigUint::one();
        assert!(!potenz_ist_brauchbar(&p, &BigUint::one()));
        assert!(!potenz_ist_brauchbar(&p, &BigUint::from(2u32)));
        assert!(!potenz_ist_brauchbar(&p, &(p.clone() - BigUint::one())));
        // 2^1500 liegt UNTER der Schranke von 2^1984 und ist deshalb
        // auch nicht brauchbar -- nur was dazwischen liegt, taugt.
        assert!(!potenz_ist_brauchbar(&p, &(BigUint::one() << 1500u32)));
        assert!(potenz_ist_brauchbar(&p, &(BigUint::one() << 2000u32)));
    }

    /// Der Fingerabdruck sind die letzten acht Bytes des SHA1, klein
    /// endend gelesen. Nachgerechnet an einem festen Wert.
    #[test]
    fn fingerabdruck_ist_das_ende_des_sha1() {
        let schluessel = vec![0u8; 256];
        let h = sha1::Sha1::digest(&schluessel);
        let mut acht = [0u8; 8];
        acht.copy_from_slice(&h[12..20]);
        assert_eq!(schluessel_fingerabdruck(&schluessel), i64::from_le_bytes(acht));
    }

    /// Die vier Pruefzeichen haengen an Schluessel UND g_a: aendert sich
    /// eines von beiden, aendern sie sich.
    #[test]
    fn pruefzeichen_haengen_an_beidem() {
        let schluessel = vec![7u8; 256];
        let g_a = vec![9u8; 256];
        let a = pruefzeichen_stellen(&schluessel, &g_a, 333);
        let mut anderer_schluessel = schluessel.clone();
        anderer_schluessel[0] ^= 1;
        let b = pruefzeichen_stellen(&anderer_schluessel, &g_a, 333);
        let mut anderes_ga = g_a.clone();
        anderes_ga[255] ^= 1;
        let c = pruefzeichen_stellen(&schluessel, &anderes_ga, 333);
        assert_ne!(a, b);
        assert_ne!(a, c);
        for s in a {
            assert!(s < 333);
        }
    }

    /// Eine Potenz muss auf 256 Bytes aufgefuellt werden, sonst stimmt
    /// der Abdruck der Gegenseite nicht.
    #[test]
    fn potenzen_sind_immer_256_bytes() {
        let klein = BigUint::from(5u32);
        assert_eq!(auf_256(&klein).len(), 256);
        assert_eq!(auf_256(&klein)[255], 5);
        assert_eq!(auf_256(&klein)[0], 0);
    }
}
