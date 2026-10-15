//! Converts an OCTGN Raw Deal `set.xml` into an OpenWWX card set.
//!
//! Usage: `octgn-import <set.xml> <set name> <id prefix> <out.ron> [--overlay scripts.ron] [--base sets-dir]`
//!
//! With `--base`, cards and Superstars the overlay does not script take the
//! script of an already imported card with the same title and text (reprints).

use std::path::Path;
use std::process::ExitCode;

use rawdeal_cards::octgn::{Overlay, import_set};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [xml, name, prefix, out, rest @ ..] = args.as_slice() else {
        eprintln!("usage: octgn-import <set.xml> <set name> <id prefix> <out.ron> [--overlay scripts.ron]");
        return ExitCode::FAILURE;
    };
    let mut overlay = Overlay::default();
    let mut base = Vec::new();
    let mut args = rest.iter();
    while let Some(flag) = args.next() {
        let Some(path) = args.next() else {
            eprintln!("unexpected arguments: {rest:?}");
            return ExitCode::FAILURE;
        };
        match flag.as_str() {
            "--overlay" => {
                match std::fs::read_to_string(path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| ron::from_str::<Overlay>(&t).map_err(|e| e.to_string()))
                {
                    Ok(o) => overlay = o,
                    Err(e) => {
                        eprintln!("{path}: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--base" => match rawdeal_cards::load_dir(Path::new(path)) {
                Ok(sets) => base = sets,
                Err(e) => {
                    eprintln!("{path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
            _ => {
                eprintln!("unexpected arguments: {rest:?}");
                return ExitCode::FAILURE;
            }
        }
    }
    let xml_text = match std::fs::read_to_string(xml) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{xml}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (mut set, report) = match import_set(&xml_text, name, prefix, &overlay) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{xml}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let key = |t: &str| t.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let mut reused = 0;
    for card in &mut set.cards {
        if overlay.scripts.contains_key(&card.title) {
            continue;
        }
        let same = base
            .iter()
            .flat_map(|s| &s.cards)
            .find(|b| b.title.eq_ignore_ascii_case(&card.title) && key(&b.text) == key(&card.text) && b.id != card.id);
        if let Some(b) = same {
            card.script = b.script.clone();
            reused += 1;
        }
    }
    for star in &mut set.superstars {
        if overlay.abilities.contains_key(&star.name) {
            continue;
        }
        let same = base
            .iter()
            .flat_map(|s| &s.superstars)
            .find(|b| b.name.eq_ignore_ascii_case(&star.name) && key(&b.text) == key(&star.text) && b.id != star.id);
        if let Some(b) = same {
            star.abilities = b.abilities.clone();
            reused += 1;
        }
    }
    if let Err(e) = rawdeal_cards::build_db([set.clone()]) {
        eprintln!("imported set is inconsistent: {e}");
        return ExitCode::FAILURE;
    }
    let config = ron::ser::PrettyConfig::new().struct_names(false);
    let text = ron::ser::to_string_pretty(&set, config).expect("card sets serialize");
    if let Err(e) = std::fs::write(Path::new(out), text) {
        eprintln!("{out}: {e}");
        return ExitCode::FAILURE;
    }

    println!("{}: {} cards, {} Superstars -> {out}", set.name, report.cards, report.superstars);
    println!("Scripts reused from earlier sets: {reused}");
    println!("Reversals with inferred targets: {}", report.reversals_inferred.len());
    for (title, targets) in &report.reversals_inferred {
        println!("  {title}: {targets}");
    }
    println!(
        "Reversals that cannot reverse yet ({}): {}",
        report.reversals_unscripted.len(),
        report.reversals_unscripted.join("; ")
    );
    for (title, kind) in &report.skipped {
        println!("Skipped {title}: unknown card type {kind:?}");
    }
    for key in &report.unused_overlay {
        println!("Overlay entry matched no card: {key}");
    }
    ExitCode::SUCCESS
}
