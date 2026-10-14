//! Converts an OCTGN Raw Deal `set.xml` into an OpenWWX card set.
//!
//! Usage: `octgn-import <set.xml> <set name> <id prefix> <out.ron> [--overlay scripts.ron]`

use std::path::Path;
use std::process::ExitCode;

use rawdeal_cards::octgn::{Overlay, import_set};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [xml, name, prefix, out, rest @ ..] = args.as_slice() else {
        eprintln!("usage: octgn-import <set.xml> <set name> <id prefix> <out.ron> [--overlay scripts.ron]");
        return ExitCode::FAILURE;
    };
    let overlay = match rest {
        [flag, path] if flag == "--overlay" => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| ron::from_str::<Overlay>(&t).map_err(|e| e.to_string()))
        {
            Ok(o) => o,
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::FAILURE;
            }
        },
        [] => Overlay::default(),
        _ => {
            eprintln!("unexpected arguments: {rest:?}");
            return ExitCode::FAILURE;
        }
    };
    let xml_text = match std::fs::read_to_string(xml) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{xml}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (set, report) = match import_set(&xml_text, name, prefix, &overlay) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{xml}: {e}");
            return ExitCode::FAILURE;
        }
    };
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
