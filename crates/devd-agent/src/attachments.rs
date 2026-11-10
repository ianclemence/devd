//! Files attached to a message, put in front of the model.
//!
//! A message that carries files says so in its own text ("Attached files:" and a path
//! for each), which is all a model without eyes can use: it may open the file with a tool.
//! A model that can see is given the pictures themselves as well, so it does not have to
//! find them first.
//!
//! Only files devd itself stored under its uploads folder are ever read, and only when
//! their first bytes really are a picture. A path that points anywhere else is left as
//! the words it is, so a message cannot make devd hand an arbitrary file to a provider.

use std::path::Path;

use base64::Engine;
use serde_json::{json, Value};

/// The largest picture sent inline. A larger one stays a path.
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// How many pictures go with one request: the most recent. Older ones stay in the
/// conversation as paths, so a long thread does not carry every screenshot forever.
const MAX_IMAGES: usize = 3;

/// What kind of picture these bytes are, by what the file really starts with.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The paths a message names after "Attached files:".
pub fn attached_paths(text: &str) -> Vec<String> {
    let Some(at) = text.find("Attached files:\n") else { return Vec::new() };
    text[at + "Attached files:\n".len()..]
        .lines()
        .take_while(|l| l.starts_with("- "))
        .map(|l| l[2..].trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn read_image(root: &Path, path: &Path) -> Option<Value> {
    let real = path.canonicalize().ok()?;
    if !real.starts_with(root) {
        return None;
    }
    let meta = std::fs::metadata(&real).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let bytes = std::fs::read(&real).ok()?;
    let mime = image_type(&bytes)?;
    Some(json!({
        "type": "image",
        "source": { "type": "base64", "media_type": mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }
    }))
}

/// The messages a model is sent, with the attached pictures in them when it can see.
pub fn with_images(messages: &[Value], vision: bool, uploads: &Path) -> Vec<Value> {
    let mut out = messages.to_vec();
    if !vision {
        return out;
    }
    let Ok(root) = uploads.canonicalize() else { return out };
    let mut room = MAX_IMAGES;
    for m in out.iter_mut().rev() {
        if room == 0 {
            break;
        }
        if m["role"] != "user" {
            continue;
        }
        let Some(blocks) = m["content"].as_array() else { continue };
        let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
        let mut images: Vec<Value> = Vec::new();
        for p in attached_paths(&text) {
            if room == 0 {
                break;
            }
            if let Some(block) = read_image(&root, Path::new(&p)) {
                images.push(block);
                room -= 1;
            }
        }
        if !images.is_empty() {
            images.extend(blocks.iter().cloned());
            m["content"] = Value::Array(images);
        }
    }
    out
}

/// Whether a prepared message list carries actual image blocks.
pub fn has_images(messages: &[Value]) -> bool {
    messages.iter().any(|m| {
        m["content"]
            .as_array()
            .map(|blocks| blocks.iter().any(|b| b["type"] == "image"))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("devd-attach-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn message(paths: &[&std::path::Path]) -> Value {
        let list = paths.iter().map(|p| format!("- {}", p.display())).collect::<Vec<_>>().join("\n");
        json!({ "role": "user", "content": [{ "type": "text", "text": format!("look\n\nAttached files:\n{list}") }] })
    }

    #[test]
    fn a_picture_is_recognised_by_its_bytes_not_its_name() {
        assert_eq!(image_type(PNG), Some("image/png"));
        assert_eq!(image_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(image_type(b"MZ\x90\0"), None, "an executable named .png is not a picture");
    }

    #[test]
    fn a_model_that_can_see_is_given_the_picture_first() {
        let d = dir("see");
        let p = d.join("a.png");
        std::fs::write(&p, PNG).unwrap();
        let out = with_images(&[message(&[&p])], true, &d);
        let blocks = out[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!(blocks[0]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["type"], "text", "the words, with the path, stay");
        assert!(has_images(&out));
    }

    #[test]
    fn has_images_is_false_for_a_text_only_turn() {
        let d = dir("textonly");
        let p = d.join("notes.txt");
        std::fs::write(&p, b"hello").unwrap();
        assert!(!has_images(&with_images(&[message(&[&p])], true, &d)));
    }

    #[test]
    fn a_model_that_cannot_see_gets_the_message_unchanged() {
        let d = dir("blind");
        let p = d.join("a.png");
        std::fs::write(&p, PNG).unwrap();
        let m = message(&[&p]);
        assert_eq!(with_images(std::slice::from_ref(&m), false, &d), vec![m]);
    }

    #[test]
    fn only_files_in_the_uploads_folder_are_read() {
        let uploads = dir("only");
        let elsewhere = dir("else");
        let p = elsewhere.join("secret.png");
        std::fs::write(&p, PNG).unwrap();
        let m = message(&[&p]);
        assert_eq!(with_images(std::slice::from_ref(&m), true, &uploads), vec![m], "a path outside uploads stays a path");
    }

    #[test]
    fn a_renamed_file_is_not_sent_as_a_picture() {
        let d = dir("fake");
        let p = d.join("not-really.png");
        std::fs::write(&p, b"MZ\x90\0 program").unwrap();
        let m = message(&[&p]);
        assert_eq!(with_images(std::slice::from_ref(&m), true, &d), vec![m]);
    }

    #[test]
    fn only_the_most_recent_pictures_go() {
        let d = dir("many");
        let mut msgs = Vec::new();
        for i in 0..5 {
            let p = d.join(format!("{i}.png"));
            std::fs::write(&p, PNG).unwrap();
            msgs.push(message(&[&p]));
        }
        let out = with_images(&msgs, true, &d);
        let with: Vec<bool> = out.iter().map(|m| m["content"][0]["type"] == "image").collect();
        assert_eq!(with, vec![false, false, true, true, true]);
    }
}
