//! Die zwanzig Befehle, die der Python-Daemon kannte.
//!
//! Namen, Argumente und Rueckgaben sind uebernommen, nicht neu erdacht --
//! die Begruendung steht in formen.rs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use grammers_client::types::{Attribute, Downloadable, InputMessage, Media};
use grammers_session::PackedChat;
use grammers_tl_types as tl;
use serde_json::{json, Value};

use crate::formen;
use crate::{datenverzeichnis, Lage};

pub async fn behandeln(lage: &Arc<Lage>, frage: &Value) -> Value {
    let befehl = frage.get("cmd").and_then(|x| x.as_str()).unwrap_or("");
    let leer = json!({});
    let args = frage.get("args").unwrap_or(&leer);

    let ergebnis = match befehl {
        "get_auth_state" => Ok(json!({"state": lage.anmeldung.lock().await.zustand})),
        "send_phone" => telefon_senden(lage, args).await,
        "send_code" => code_senden(lage, args).await,
        "send_2fa" => zweitfaktor(lage, args).await,
        "logout" => abmelden(lage).await,
        "get_dialogs" => dialoge(lage, args).await,
        "get_messages" => nachrichten(lage, args).await,
        "search_messages" => suchen(lage, args).await,
        "find_user" => benutzer_finden(lage, args).await,
        "send_message" => senden(lage, args).await,
        "send_file" => datei_senden(lage, args).await,
        "download_media" => herunterladen(lage, args).await,
        "get_avatar" => avatar(lage, args).await,
        "mark_read" => gelesen(lage, args).await,
        "set_typing" => tippen(lage, args).await,
        "get_contacts" => kontakte(lage).await,
        "get_settings" => Ok(json!({"settings": *lage.einstellungen.lock().await})),
        "set_setting" => einstellung_setzen(lage, args).await,
        "get_me" => ich(lage).await,
        _ => return json!({"error": format!("Unknown command: {befehl}")}),
    };

    // Was sich unterwegs an Chats angesammelt hat, kommt auf die Platte.
    lage.verzeichnis.sichern();

    match ergebnis {
        Ok(v) => v,
        Err(e) => {
            eprintln!("⚠ {befehl}: {e}");
            json!({"error": e})
        }
    }
}

// --- Anmeldung --------------------------------------------------------

async fn telefon_senden(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let telefon = args.get("phone").and_then(|x| x.as_str()).unwrap_or("");
    match lage.client.request_login_code(telefon).await {
        Ok(marke) => {
            let mut a = lage.anmeldung.lock().await;
            a.anmeldemarke = Some(marke);
            a.telefon = telefon.to_string();
            a.zustand = "need_code".into();
            Ok(json!({"ok": true, "state": "need_code"}))
        }
        Err(e) => Ok(json!({"ok": false, "error": e.to_string()})),
    }
}

async fn code_senden(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let code = args.get("code").and_then(|x| x.as_str()).unwrap_or("");
    let mut a = lage.anmeldung.lock().await;
    let Some(marke) = a.anmeldemarke.as_ref() else {
        return Ok(json!({"ok": false, "error": "kein Anmeldevorgang offen"}));
    };
    match lage.client.sign_in(marke, code).await {
        Ok(_) => {
            a.zustand = "ready".into();
            drop(a);
            sitzung_sichern(lage).await;
            Ok(json!({"ok": true, "state": "ready"}))
        }
        Err(grammers_client::SignInError::PasswordRequired(marke)) => {
            a.passwortmarke = Some(marke);
            a.zustand = "need_2fa".into();
            Ok(json!({"ok": true, "state": "need_2fa"}))
        }
        Err(e) => Ok(json!({"ok": false, "error": e.to_string()})),
    }
}

async fn zweitfaktor(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let passwort = args.get("password").and_then(|x| x.as_str()).unwrap_or("");
    let mut a = lage.anmeldung.lock().await;
    let Some(marke) = a.passwortmarke.take() else {
        return Ok(json!({"ok": false, "error": "kein Passwort verlangt"}));
    };
    match lage.client.check_password(marke, passwort).await {
        Ok(_) => {
            a.zustand = "ready".into();
            drop(a);
            sitzung_sichern(lage).await;
            Ok(json!({"ok": true, "state": "ready"}))
        }
        Err(e) => Ok(json!({"ok": false, "error": e.to_string()})),
    }
}

async fn abmelden(lage: &Arc<Lage>) -> Result<Value, String> {
    lage.client.sign_out().await.map_err(|e| e.to_string())?;
    lage.anmeldung.lock().await.zustand = "need_phone".into();
    Ok(json!({"ok": true}))
}

/// Nach einer Anmeldung steht die eigene Kennung fest -- und sie gehoert
/// in die Sitzungsdatei, sonst waehlt der naechste Start wieder das
/// falsche Rechenzentrum.
async fn sitzung_sichern(lage: &Arc<Lage>) {
    if let Ok(ich) = lage.client.get_me().await {
        let dc = lage.client.session().get_user().map(|u| u.dc).unwrap_or(2);
        crate::sitzung::benutzer_festhalten(&lage.client, &datenverzeichnis(), ich.id(), dc);
    }
}

// --- Chats und Nachrichten -------------------------------------------

pub async fn dialoge_holen(lage: &Arc<Lage>, grenze: usize, versatz: usize) -> Result<Vec<Value>, String> {
    let mut lauf = lage.client.iter_dialogs();
    let mut aus = Vec::new();
    let mut gesehen = 0usize;
    while let Some(d) = lauf.next().await.map_err(|e| e.to_string())? {
        lage.verzeichnis.merken(&d.chat().pack());
        if gesehen >= versatz {
            aus.push(formen::dialog(&d));
        }
        gesehen += 1;
        if aus.len() >= grenze {
            break;
        }
    }
    lage.verzeichnis.sichern();
    Ok(aus)
}

async fn dialoge(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let grenze = args.get("limit").and_then(|x| x.as_u64()).unwrap_or(30) as usize;
    let versatz = args.get("offset").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    let liste = dialoge_holen(lage, grenze, versatz).await?;
    Ok(json!({"dialogs": liste}))
}

/// Von der markierten Kennung zum Chat, den MTProto versteht.
///
/// Steht er nicht im Verzeichnis, wird einmal ueber die Dialoge gegangen;
/// das kostet Zeit, kommt aber nur beim allerersten Mal vor.
async fn chat(lage: &Arc<Lage>, kennung: i64) -> Result<PackedChat, String> {
    if let Some(c) = lage.verzeichnis.finden(kennung) {
        return Ok(c);
    }
    eprintln!("== {kennung} unbekannt, gehe die Dialoge durch");
    let _ = dialoge_holen(lage, 1000, 0).await;
    lage.verzeichnis
        .finden(kennung)
        .ok_or_else(|| format!("Chat {kennung} nicht gefunden"))
}

fn kennung_aus(args: &Value) -> Result<i64, String> {
    args.get("chat_id")
        .and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
        .ok_or_else(|| "chat_id fehlt".to_string())
}

async fn nachrichten(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let c = chat(lage, kennung_aus(args)?).await?;
    let grenze = args.get("limit").and_then(|x| x.as_u64()).unwrap_or(20) as usize;
    let ab = args.get("offset").and_then(|x| x.as_i64()).unwrap_or(0) as i32;

    let mut lauf = lage.client.iter_messages(c);
    if ab > 0 {
        lauf = lauf.offset_id(ab);
    }
    let mut aus = Vec::new();
    while let Some(m) = lauf.next().await.map_err(|e| e.to_string())? {
        aus.push(formen::nachricht(&m));
        if aus.len() >= grenze {
            break;
        }
    }
    Ok(json!({"messages": aus}))
}

async fn suchen(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let frage = args.get("query").and_then(|x| x.as_str()).unwrap_or("");
    if frage.is_empty() {
        return Ok(json!({"results": []}));
    }
    let grenze = args.get("limit").and_then(|x| x.as_u64()).unwrap_or(20) as usize;
    let mut aus = Vec::new();

    if let Ok(kennung) = kennung_aus(args) {
        let c = chat(lage, kennung).await?;
        let mut lauf = lage.client.search_messages(c).query(frage);
        while let Some(m) = lauf.next().await.map_err(|e| e.to_string())? {
            aus.push(treffer(&m));
            if aus.len() >= grenze {
                break;
            }
        }
    } else {
        let mut lauf = lage.client.search_all_messages().query(frage);
        while let Some(m) = lauf.next().await.map_err(|e| e.to_string())? {
            lage.verzeichnis.merken(&m.chat().pack());
            aus.push(treffer(&m));
            if aus.len() >= grenze {
                break;
            }
        }
    }
    Ok(json!({"results": aus}))
}

fn treffer(m: &grammers_client::types::Message) -> Value {
    let mut v = formen::nachricht(m);
    v["chat_title"] = json!(m.chat().name());
    v["chat_id_str"] = json!(v["chat_id"].as_i64().unwrap_or(0).to_string());
    v
}

async fn benutzer_finden(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let frage = args.get("query").and_then(|x| x.as_str()).unwrap_or("").trim();
    if frage.is_empty() {
        return Ok(json!({"user": null, "error": "Empty query"}));
    }

    // Nummern: der Server loest sie nur ueber contacts.resolvePhone auf.
    let nur_ziffern = frage.trim_start_matches('+').chars().all(|c| c.is_ascii_digit());
    if nur_ziffern {
        let nummer = frage.trim_start_matches('+').to_string();
        match lage
            .client
            .invoke(&tl::functions::contacts::ResolvePhone { phone: nummer })
            .await
        {
            Ok(tl::enums::contacts::ResolvedPeer::Peer(p)) => {
                for u in &p.users {
                    let u = grammers_client::types::User::from_raw(u.clone());
                    lage.verzeichnis.merken(&u.pack());
                    return Ok(json!({"user": formen::benutzer(&u)}));
                }
                return Ok(json!({"user": null}));
            }
            Err(e) => return Ok(json!({"user": null, "error": e.to_string()})),
        }
    }

    let name = frage.trim_start_matches('@');
    match lage.client.resolve_username(name).await {
        Ok(Some(c)) => {
            lage.verzeichnis.merken(&c.pack());
            Ok(json!({"user": formen::chat_als_benutzer(&c)}))
        }
        Ok(None) => Ok(json!({"user": null})),
        Err(e) => Ok(json!({"user": null, "error": e.to_string()})),
    }
}

async fn senden(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let c = chat(lage, kennung_aus(args)?).await?;
    let text = args.get("text").and_then(|x| x.as_str()).unwrap_or("");
    let antwort_auf = args.get("reply_to").and_then(|x| x.as_i64()).map(|x| x as i32);

    let mut nachricht = InputMessage::text(text);
    if antwort_auf.is_some() {
        nachricht = nachricht.reply_to(antwort_auf);
    }
    let m = lage
        .client
        .send_message(c, nachricht)
        .await
        .map_err(|e| e.to_string())?;
    Ok(formen::nachricht(&m))
}

async fn datei_senden(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let c = chat(lage, kennung_aus(args)?).await?;
    let pfad = args.get("path").and_then(|x| x.as_str()).ok_or("path fehlt")?;
    let beschriftung = args.get("caption").and_then(|x| x.as_str()).unwrap_or("");
    let sprache = args.get("voice").and_then(|x| x.as_bool()).unwrap_or(false);

    let hochgeladen = lage
        .client
        .upload_file(pfad)
        .await
        .map_err(|e| e.to_string())?;

    let name = Path::new(pfad)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "datei".into());

    let mut nachricht = InputMessage::text(beschriftung).file(hochgeladen);
    if sprache {
        nachricht = nachricht
            .mime_type("audio/ogg")
            .attribute(Attribute::Voice {
                duration: std::time::Duration::from_secs(0),
                waveform: None,
            });
    } else {
        nachricht = nachricht.attribute(Attribute::FileName(name));
    }

    let m = lage
        .client
        .send_message(c, nachricht)
        .await
        .map_err(|e| e.to_string())?;
    Ok(formen::nachricht(&m))
}

async fn herunterladen(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let c = chat(lage, kennung_aus(args)?).await?;
    let kennung = args.get("msg_id").and_then(|x| x.as_i64()).ok_or("msg_id fehlt")? as i32;

    let mut gefunden = lage
        .client
        .get_messages_by_id(c, &[kennung])
        .await
        .map_err(|e| e.to_string())?;
    let Some(Some(m)) = gefunden.pop() else {
        return Ok(json!({"path": null}));
    };
    let Some(medium) = m.media() else {
        return Ok(json!({"path": null}));
    };

    let name = match &medium {
        Media::Document(d) if !d.name().is_empty() => d.name().to_string(),
        Media::Photo(_) | Media::Sticker(_) => format!("photo_{kennung}.jpg"),
        _ => format!("datei_{kennung}"),
    };

    let ziel = datenverzeichnis().join("downloads").join(&name);
    lage.client
        .download_media(&Downloadable::Media(medium), &ziel)
        .await
        .map_err(|e| e.to_string())?;

    // Der Python-Daemon legte eine Kopie nach MyDocs/Downloads, damit die
    // Datei auch dann noch da ist, wenn der Zwischenspeicher aufgeraeumt
    // wird -- und damit sie ueber USB zu sehen ist.
    match nach_mydocs(&ziel, &name) {
        Some(p) => Ok(json!({"path": p})),
        None => Ok(json!({"path": ziel.to_string_lossy()})),
    }
}

fn nach_mydocs(quelle: &Path, name: &str) -> Option<String> {
    let heim = std::env::var("HOME").ok()?;
    let ordner = PathBuf::from(heim).join("MyDocs/Downloads");
    std::fs::create_dir_all(&ordner).ok()?;
    let mut ziel = ordner.join(name);
    let mut n = 1;
    while ziel.exists() {
        let stamm = Path::new(name).file_stem()?.to_string_lossy().to_string();
        let endung = Path::new(name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        ziel = ordner.join(format!("{stamm}_{n}{endung}"));
        n += 1;
    }
    std::fs::copy(quelle, &ziel).ok()?;
    Some(ziel.to_string_lossy().to_string())
}

async fn avatar(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let kennung = args
        .get("entity_id")
        .and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
        .ok_or("entity_id fehlt")?;
    let erzwingen = args.get("force").and_then(|x| x.as_bool()).unwrap_or(false);

    let ordner = datenverzeichnis().join("cache/avatars");
    let ziel = ordner.join(format!("{kennung}.jpg"));
    if !erzwingen && ziel.exists() {
        return Ok(json!({"path": ziel.to_string_lossy()}));
    }

    let c = chat(lage, kennung).await?;
    let voll = lage.client.unpack_chat(c).await.map_err(|e| e.to_string())?;
    let Some(bild) = voll.photo_downloadable(false) else {
        return Ok(json!({"path": null}));
    };
    lage.client
        .download_media(&bild, &ziel)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({"path": ziel.to_string_lossy()}))
}

async fn gelesen(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let c = chat(lage, kennung_aus(args)?).await?;
    lage.client.mark_as_read(c).await.map_err(|e| e.to_string())?;
    Ok(json!({"ok": true}))
}

async fn tippen(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let c = chat(lage, kennung_aus(args)?).await?;
    let tippt = args.get("typing").and_then(|x| x.as_bool()).unwrap_or(true);
    let handlung = lage.client.action(c);
    let ergebnis = if tippt {
        handlung
            .oneshot(tl::types::SendMessageTypingAction {})
            .await
    } else {
        handlung.cancel().await
    };
    ergebnis.map_err(|e| e.to_string())?;
    Ok(json!({"ok": true}))
}

async fn kontakte(lage: &Arc<Lage>) -> Result<Value, String> {
    let antwort = lage
        .client
        .invoke(&tl::functions::contacts::GetContacts { hash: 0 })
        .await
        .map_err(|e| e.to_string())?;
    let mut aus = Vec::new();
    if let tl::enums::contacts::Contacts::Contacts(c) = antwort {
        for u in c.users {
            let u = grammers_client::types::User::from_raw(u);
            lage.verzeichnis.merken(&u.pack());
            aus.push(formen::benutzer(&u));
        }
    }
    Ok(json!({"contacts": aus}))
}

async fn ich(lage: &Arc<Lage>) -> Result<Value, String> {
    let u = lage.client.get_me().await.map_err(|e| e.to_string())?;
    lage.verzeichnis.merken(&u.pack());
    Ok(json!({"user": formen::benutzer(&u)}))
}

// --- Einstellungen ----------------------------------------------------

/// Dieselben Vorgaben wie im Python-Daemon: sie sind auf 2G gerechnet.
pub fn einstellungen_laden(daten: &Path) -> Value {
    let mut vorgabe = json!({
        "dark_mode": true,
        "auto_download_photos": false,
        "auto_download_photos_limit": 102400,
        "auto_download_voice": true,
        "auto_download_voice_limit": 307200,
        "auto_download_videos": false,
        "load_avatars": true,
        "link_preview": false,
        "compress_uploads": true,
        "upload_quality": 70,
        "max_upload_size": 1280,
        "preload_messages": 20,
        "cache_messages": true,
        "dialogs_limit": 30,
        "messages_limit": 20
    });
    if let Ok(text) = std::fs::read_to_string(daten.join("settings.json")) {
        if let Ok(Value::Object(gespeichert)) = serde_json::from_str::<Value>(&text) {
            if let Value::Object(v) = &mut vorgabe {
                for (k, w) in gespeichert {
                    v.insert(k, w);
                }
            }
        }
    }
    vorgabe
}

async fn einstellung_setzen(lage: &Arc<Lage>, args: &Value) -> Result<Value, String> {
    let schluessel = args.get("key").and_then(|x| x.as_str()).ok_or("key fehlt")?;
    let wert = args.get("value").cloned().unwrap_or(Value::Null);
    let mut e = lage.einstellungen.lock().await;
    if let Value::Object(m) = &mut *e {
        if m.contains_key(schluessel) {
            m.insert(schluessel.to_string(), wert);
            if let Ok(j) = serde_json::to_string(&*e) {
                let _ = std::fs::write(datenverzeichnis().join("settings.json"), j);
            }
        }
    }
    Ok(json!({"ok": true}))
}
