//! OCTGN image packs (`.o8c`): zip files holding the scanned cards as
//! `<game>/Sets/<set>/Cards/<card GUID>.jpg`. Importing one copies its images
//! into the card folder, where [`crate::art`] finds them by GUID.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Copies every image in the packs into `dest` (created if needed), keeping
/// only the file names. Returns how many images were copied.
pub fn extract(packs: &[impl AsRef<Path>], dest: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("No se pudo crear {}: {e}", dest.display()))?;
    let mut copied = 0;
    for pack in packs {
        let pack = pack.as_ref();
        let file = File::open(pack).map_err(|e| format!("No se pudo abrir {}: {e}", pack.display()))?;
        let mut zip = zip::ZipArchive::new(BufReader::new(file))
            .map_err(|e| format!("{} no es un paquete de imágenes válido: {e}", pack.display()))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| format!("{}: {e}", pack.display()))?;
            if entry.is_dir() {
                continue;
            }
            // `enclosed_name` drops entries that would escape the folder.
            let Some(name) = entry.enclosed_name().and_then(|p| p.file_name().map(|n| n.to_owned())) else {
                continue;
            };
            let ext = Path::new(&name).extension().and_then(|e| e.to_str()).map(str::to_lowercase);
            if !ext.is_some_and(|e| EXTENSIONS.contains(&e.as_str())) {
                continue;
            }
            let path = dest.join(&name);
            let mut out = BufWriter::new(
                File::create(&path).map_err(|e| format!("No se pudo escribir {}: {e}", path.display()))?,
            );
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("No se pudo escribir {}: {e}", path.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn images_are_copied_flat_by_file_name() {
        let dir = std::env::temp_dir().join(format!("openwwx-o8c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pack = dir.join("pack.o8c");
        let mut zip = zip::ZipWriter::new(File::create(&pack).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.add_directory("game/Sets/set/Cards/", options).unwrap();
        for (name, body) in [
            ("game/Sets/set/Cards/46B50940-69d6.jpg", "a"),
            ("game/Sets/set/Cards/other.png", "b"),
            ("game/Sets/set/set.xml", "not an image"),
            ("../escape.jpg", "outside"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        let art = dir.join("art");
        assert_eq!(extract(&[&pack], &art), Ok(2));
        assert_eq!(std::fs::read_to_string(art.join("46B50940-69d6.jpg")).unwrap(), "a");
        assert!(art.join("other.png").exists());
        assert!(!art.join("set.xml").exists());
        assert!(!dir.join("escape.jpg").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `OPENWWX_O8C=<pack> OPENWWX_O8C_OUT=<folder> cargo test -p openwwx -- --ignored real_pack`
    #[test]
    #[ignore]
    fn real_pack() {
        let pack = std::env::var("OPENWWX_O8C").expect("OPENWWX_O8C");
        let out = std::env::var("OPENWWX_O8C_OUT").expect("OPENWWX_O8C_OUT");
        let n = extract(&[pack], Path::new(&out)).unwrap();
        println!("{n} images copied");
        assert!(n > 0);
        let bad: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            // Like `art`, which reads the format from the bytes (one ".jpg" is a PNG).
            .filter(|e| std::fs::read(e.path()).map_or(true, |b| image::load_from_memory(&b).is_err()))
            .map(|e| e.file_name())
            .collect();
        assert!(bad.is_empty(), "images that do not load: {bad:?}");
    }
}
