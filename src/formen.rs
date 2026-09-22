//! Die Form der Antworten -- und das ist ein Vertrag, kein Geschmack.
//!
//! Auf der anderen Seite des Sockets sitzen zwei Programme, die niemand
//! anfasst: die QML-Oberflaeche von PyTeleGram und die
//! Nachrichtenbruecke. Beide lesen genau die Felder, die
//! telegram_daemon.py geschrieben hat. Ein anderer Name, ein anderer
//! Typ, eine andere Rechenart fuer die Chat-Kennung -- und die
//! Nachrichten-App legt neue, leere Gespraeche an, statt die
//! vorhandenen zu finden.
//!
//! Deshalb steht hier alles so, wie Telethon es geschrieben haette,
//! einschliesslich der Eigenheiten: `date` ist eine Fliesskommazahl,
//! `sender_name` traegt nur den Vornamen, `chat_id` ist die markierte
//! Kennung.

use grammers_client::types::{Chat, Dialog, Media, Message, User};
use grammers_tl_types as tl;
use serde_json::{json, Value};

use crate::verzeichnis::markiert;

pub fn nachricht(m: &Message) -> Value {
    let mut v = json!({
        "id": m.id(),
        "chat_id": markiert(&m.chat().pack()),
        "date": m.date().timestamp() as f64,
        "text": m.text(),
        "out": m.outgoing(),
        "reply_to": m.reply_to_message_id(),
        "has_media": false,
        "media_type": "",
        "media_filename": "",
    });

    if let Some(medium) = m.media() {
        v["has_media"] = json!(true);
        match &medium {
            Media::Photo(_) => {
                v["media"] = json!({"type": "photo"});
                v["media_type"] = json!("photo");
            }
            Media::Sticker(_) => {
                v["media"] = json!({"type": "photo"});
                v["media_type"] = json!("photo");
            }
            Media::Document(d) => {
                let art = dokumentart(d);
                v["media"] = json!({
                    "type": art,
                    "size": d.size(),
                    "filename": d.name(),
                });
                v["media_type"] = json!(art);
                v["media_filename"] = json!(d.name());
            }
            _ => {
                v["media"] = json!({"type": "document"});
                v["media_type"] = json!("document");
            }
        }
    }

    match m.sender() {
        Some(s) => {
            v["sender_id"] = json!(s.id());
            v["sender_name"] = json!(vorname(&s));
        }
        None => {
            v["sender_id"] = json!(0);
            v["sender_name"] = json!("");
        }
    }
    v
}

/// Sprachnachricht, Videokreis oder gewoehnliche Datei -- der
/// Unterschied steckt in den Attributen des Dokuments.
fn dokumentart(d: &grammers_client::types::media::Document) -> &'static str {
    let tl::enums::Document::Document(roh) = (match d.raw.document.as_ref() {
        Some(x) => x,
        None => return "document",
    }) else {
        return "document";
    };
    for a in &roh.attributes {
        match a {
            tl::enums::DocumentAttribute::Audio(x) if x.voice => return "voice",
            tl::enums::DocumentAttribute::Video(x) if x.round_message => return "video_note",
            _ => {}
        }
    }
    "document"
}

/// Telethon schrieb in sender_name nur den Vornamen; bei Gruppen und
/// Kanaelen den Titel.
fn vorname(c: &Chat) -> String {
    match c {
        Chat::User(u) => u.first_name().to_string(),
        andere => andere.name().to_string(),
    }
}

pub fn dialog(d: &Dialog) -> Value {
    let chat = d.chat();
    let (unread, stumm) = aus_rohem(&d.raw);

    let letzte = d.last_message.as_ref().map(|m| {
        let mut text = m.text().to_string();
        if text.is_empty() {
            if let Some(medium) = m.media() {
                text = match medium {
                    Media::Photo(_) | Media::Sticker(_) => "📷 Photo".into(),
                    _ => "📎 File".into(),
                };
            }
        }
        json!({
            "text": text,
            "date": m.date().timestamp() as f64,
            "out": m.outgoing(),
        })
    });

    let mut v = json!({
        "id": markiert(&chat.pack()),
        "id_str": markiert(&chat.pack()).to_string(),
        "unread": unread,
        "muted": stumm,
        "last_message": letzte,
    });

    match chat {
        Chat::User(u) => {
            v["type"] = json!("user");
            v["first_name"] = json!(u.first_name());
            v["last_name"] = json!(u.last_name().unwrap_or(""));
            let voll = u.full_name();
            v["title"] = json!(if voll.trim().is_empty() { "Unknown".into() } else { voll });
            v["status"] = json!(zustand(u));
        }
        Chat::Group(_) => {
            v["type"] = json!("group");
            v["title"] = json!(chat.name());
            v["first_name"] = json!("");
            v["last_name"] = json!("");
        }
        Chat::Channel(_) => {
            v["type"] = json!("channel");
            v["title"] = json!(chat.name());
            v["first_name"] = json!("");
            v["last_name"] = json!("");
        }
    }
    v
}

/// Ungelesene und Stummschaltung stecken im rohen Dialog, nicht im Chat.
fn aus_rohem(roh: &tl::enums::Dialog) -> (i32, bool) {
    let d = match roh {
        tl::enums::Dialog::Dialog(d) => d,
        // Ordnerdialoge tragen keine eigene Zaehlung, die uns hilft.
        tl::enums::Dialog::Folder(_) => return (0, false),
    };
    let tl::enums::PeerNotifySettings::Settings(n) = &d.notify_settings;
    let jetzt = chrono::Utc::now().timestamp() as i32;
    let stumm = n.silent.unwrap_or(false) || n.mute_until.map(|t| t > jetzt).unwrap_or(false);
    (d.unread_count, stumm)
}

pub fn benutzer(u: &User) -> Value {
    json!({
        "id": u.id(),
        "first_name": u.first_name(),
        "last_name": u.last_name().unwrap_or(""),
        "username": u.username().unwrap_or(""),
        "phone": u.phone().unwrap_or(""),
        "status": zustand(u),
    })
}

/// Ein Chat, der auch eine Gruppe sein kann -- find_user liefert beides.
pub fn chat_als_benutzer(c: &Chat) -> Value {
    match c {
        Chat::User(u) => benutzer(u),
        andere => json!({
            "id": andere.id(),
            "first_name": andere.name(),
            "last_name": "",
            "username": andere.username().unwrap_or(""),
            "phone": "",
            "status": "offline",
        }),
    }
}

fn zustand(u: &User) -> &'static str {
    match u.status() {
        tl::enums::UserStatus::Online(_) => "online",
        _ => "offline",
    }
}
