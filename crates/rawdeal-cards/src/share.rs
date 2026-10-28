//! Sharing decks: short text codes (`OWX1-…`) and OCTGN `.o8d` deck files.
//!
//! A code is the deck as lines of card ids, compressed and written in
//! URL-safe base64, so it only works with the same sets as the deck's
//! author, like online play.

use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use rawdeal_core::{CardDb, DeckList};

const PREFIX: &str = "OWX1-";

/// The OCTGN game id of Raw Deal, from its set files.
pub const OCTGN_GAME: &str = "dd9d1122-c855-40e6-a67f-21fd5bef2192";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareError {
    /// Not an `OWX1-` code, or a damaged one.
    BadCode,
    /// The code is from a newer version of OpenWWX.
    NewerCode,
    /// Not an OCTGN deck file.
    BadO8d(String),
    /// The OCTGN deck has no Superstar we know.
    NoSuperstar,
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShareError::BadCode => write!(f, "El código no es un mazo de OpenWWX o está incompleto."),
            ShareError::NewerCode => write!(f, "El código es de una versión más nueva de OpenWWX."),
            ShareError::BadO8d(e) => write!(f, "El archivo no es un mazo de OCTGN: {e}"),
            ShareError::NoSuperstar => write!(f, "El mazo de OCTGN no tiene una Superstar de tus sets."),
        }
    }
}

impl std::error::Error for ShareError {}

/// Consecutive copies of the same card as (count, id).
fn runs(ids: &[String]) -> Vec<(usize, &str)> {
    let mut out: Vec<(usize, &str)> = vec![];
    for id in ids {
        match out.last_mut() {
            Some((n, last)) if *last == id.as_str() => *n += 1,
            _ => out.push((1, id)),
        }
    }
    out
}

/// The deck as a code to copy and paste.
pub fn deck_code(deck: &DeckList) -> String {
    let mut text = format!("S {}\n", deck.superstar);
    for (tag, list) in [("A", &deck.arsenal), ("B", &deck.backlash), ("K", &deck.backstage)] {
        let mut sorted = list.clone();
        sorted.sort();
        for (n, id) in runs(&sorted) {
            text.push_str(&format!("{tag} {n} {id}\n"));
        }
    }
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(text.as_bytes()).expect("writing to memory");
    let bytes = encoder.finish().expect("writing to memory");
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Reads a deck code. Spaces and line breaks in it are ignored, so a code
/// split over several lines still works.
pub fn read_deck_code(code: &str) -> Result<DeckList, ShareError> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(body) = code.strip_prefix(PREFIX) else {
        return Err(match code.strip_prefix("OWX").and_then(|r| r.split('-').next()) {
            Some(v) if v.parse::<u32>().is_ok_and(|v| v > 1) => ShareError::NewerCode,
            _ => ShareError::BadCode,
        });
    };
    let bytes = URL_SAFE_NO_PAD.decode(body).map_err(|_| ShareError::BadCode)?;
    let mut text = String::new();
    DeflateDecoder::new(&bytes[..]).take(1 << 20).read_to_string(&mut text).map_err(|_| ShareError::BadCode)?;
    let mut deck = DeckList::default();
    for line in text.lines() {
        let mut parts = line.splitn(3, ' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("S"), Some(id), None) => deck.superstar = id.to_string(),
            (Some(tag @ ("A" | "B" | "K")), Some(n), Some(id)) => {
                let n: usize = n.parse().map_err(|_| ShareError::BadCode)?;
                if n > 100 {
                    return Err(ShareError::BadCode);
                }
                let list = match tag {
                    "A" => &mut deck.arsenal,
                    "B" => &mut deck.backlash,
                    _ => &mut deck.backstage,
                };
                list.extend(std::iter::repeat_n(id.to_string(), n));
            }
            _ => return Err(ShareError::BadCode),
        }
    }
    if deck.superstar.is_empty() {
        return Err(ShareError::BadCode);
    }
    Ok(deck)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The deck as an OCTGN `.o8d` file. Cards without an OCTGN id are left
/// out and returned by title.
pub fn deck_to_o8d(db: &CardDb, deck: &DeckList) -> (String, Vec<String>) {
    let mut missing = vec![];
    let mut xml =
        format!("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n<deck game=\"{OCTGN_GAME}\">\n");
    let star = db.superstar_by_id(&deck.superstar).map(|i| db.superstar(i));
    xml.push_str("  <section name=\"Superstar\" shared=\"False\">\n");
    match star {
        Some(s) if s.source_id.is_some() => xml.push_str(&format!(
            "    <card qty=\"1\" id=\"{}\">{}</card>\n",
            escape(s.source_id.as_deref().unwrap_or_default()),
            escape(&s.name)
        )),
        Some(s) => missing.push(s.name.clone()),
        None => missing.push(deck.superstar.clone()),
    }
    xml.push_str("  </section>\n");
    for (name, list) in [("Arsenal", &deck.arsenal), ("Backlash", &deck.backlash), ("Backstage", &deck.backstage)] {
        xml.push_str(&format!("  <section name=\"{name}\" shared=\"False\">\n"));
        let mut sorted = list.clone();
        sorted.sort();
        for (n, id) in runs(&sorted) {
            match db.card_by_id(id).map(|i| db.card(i)) {
                Some(def) if def.source_id.is_some() => xml.push_str(&format!(
                    "    <card qty=\"{n}\" id=\"{}\">{}</card>\n",
                    escape(def.source_id.as_deref().unwrap_or_default()),
                    escape(&def.title)
                )),
                Some(def) => missing.push(def.title.clone()),
                None => missing.push(id.to_string()),
            }
        }
        xml.push_str("  </section>\n");
    }
    xml.push_str("</deck>\n");
    (xml, missing)
}

/// Reads an OCTGN `.o8d` deck: every card goes where its kind says
/// (Superstar, Backlash, Backstage or Arsenal), whatever section it is in.
/// Cards not in `db` are left out and returned by name.
pub fn deck_from_o8d(db: &CardDb, xml: &str) -> Result<(DeckList, Vec<String>), ShareError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| ShareError::BadO8d(e.to_string()))?;
    if !doc.root_element().has_tag_name("deck") {
        return Err(ShareError::BadO8d("falta <deck>".into()));
    }
    let star_of = |guid: &str| db.superstars().iter().find(|s| s.source_id.as_deref() == Some(guid));
    let card_of = |guid: &str| db.cards().iter().find(|c| c.source_id.as_deref() == Some(guid));
    let mut deck = DeckList::default();
    let mut missing = vec![];
    for card in doc.descendants().filter(|n| n.has_tag_name("card")) {
        let guid = card.attribute("id").unwrap_or_default().to_lowercase();
        let qty: usize = card.attribute("qty").and_then(|q| q.parse().ok()).unwrap_or(1).min(100);
        let name = card.text().unwrap_or_default().trim().to_string();
        if let Some(s) = star_of(&guid) {
            deck.superstar = s.id.clone();
        } else if let Some(def) = card_of(&guid) {
            let list = if def.is_backstage() {
                &mut deck.backstage
            } else if def.is_backlash() {
                &mut deck.backlash
            } else {
                &mut deck.arsenal
            };
            list.extend(std::iter::repeat_n(def.id.clone(), qty));
        } else {
            missing.push(if name.is_empty() { guid } else { name });
        }
    }
    if deck.superstar.is_empty() {
        return Err(ShareError::NoSuperstar);
    }
    Ok((deck, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        let db = crate::test_db();
        for s in db.superstars() {
            let deck = crate::test_deck(&s.id);
            let code = deck_code(&deck);
            assert!(code.starts_with(PREFIX) && code.len() < 600, "{code}");
            let mut back = read_deck_code(&code).unwrap();
            let mut want = deck.clone();
            for d in [&mut back, &mut want] {
                d.arsenal.sort();
                d.backlash.sort();
                d.backstage.sort();
            }
            assert_eq!(back, want);
            // Line breaks and spaces from pasting don't matter.
            let (a, b) = code.split_at(30);
            assert_eq!(read_deck_code(&format!("  {a}\n {b} ")).unwrap().arsenal.len(), deck.arsenal.len());
        }
    }

    #[test]
    fn bad_codes_are_refused() {
        assert_eq!(read_deck_code("hola"), Err(ShareError::BadCode));
        assert_eq!(read_deck_code("OWX1-%%%"), Err(ShareError::BadCode));
        assert_eq!(read_deck_code("OWX2-abc"), Err(ShareError::NewerCode));
        let code = deck_code(&crate::test_deck(&crate::test_db().superstars()[0].id));
        assert_eq!(read_deck_code(&code[..code.len() / 2]), Err(ShareError::BadCode));
    }

    #[test]
    fn o8d_round_trip() {
        let mut db = crate::test_db();
        let star = db.superstars()[0].id.clone();
        let deck = crate::test_deck(&star);
        // The test set has no OCTGN ids: make some up.
        let mut cards = db.cards().to_vec();
        for (i, c) in cards.iter_mut().enumerate() {
            c.source_id = Some(format!("00000000-0000-0000-0000-{i:012}"));
        }
        let mut stars = db.superstars().to_vec();
        for (i, s) in stars.iter_mut().enumerate() {
            s.source_id = Some(format!("11111111-0000-0000-0000-{i:012}"));
        }
        db = CardDb::new(cards, stars).unwrap();
        let (xml, missing) = deck_to_o8d(&db, &deck);
        assert!(missing.is_empty(), "{missing:?}");
        assert!(xml.contains(OCTGN_GAME));
        let (mut back, missing) = deck_from_o8d(&db, &xml).unwrap();
        assert!(missing.is_empty(), "{missing:?}");
        let mut want = deck.clone();
        for d in [&mut back, &mut want] {
            d.arsenal.sort();
            d.backlash.sort();
        }
        assert_eq!(back, want);
        let unknown = xml.replacen("00000000-", "99999999-", 1);
        assert_eq!(deck_from_o8d(&db, &unknown).unwrap().1.len(), 1);
    }
}
