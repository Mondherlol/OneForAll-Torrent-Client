//! Analyse d'un disque (façon SpaceSniffer) : arbre des dossiers et gros fichiers avec leur taille.
//! Parcours en parallèle ; les éléments trop petits pour être vus sont regroupés (« rest »).

use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Catégories de fichiers (indices de `cats`)
/// 0 vidéo, 1 audio, 2 image, 3 archive, 4 programme, 5 jeu (ROM / données), 6 document, 7 autre
fn cat(name: &str) -> usize {
    let x = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match x.as_str() {
        "mkv" | "mp4" | "avi" | "mov" | "m4v" | "ts" | "m2ts" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "vob" => 0,
        "mp3" | "flac" | "wav" | "m4a" | "ogg" | "opus" | "aac" | "wma" | "alac" => 1,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "raw" | "cr2" | "nef" | "arw" | "psd" | "tif" | "tiff" | "bmp" => 2,
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "zst" | "cab" | "msi" | "torrent" => 3,
        "exe" | "dll" | "sys" | "so" | "lib" | "node" | "jar" | "appx" | "msix" => 4,
        "iso" | "cso" | "nsp" | "xci" | "nds" | "3ds" | "cia" | "gba" | "rvz" | "wbfs" | "chd" | "pak" | "pck" | "bundle" | "assets" | "vpk" | "bsa" | "ba2" | "uasset" | "ucas" | "utoc" | "forge" | "big" | "arc" | "wad" => 5,
        "pdf" | "epub" | "cbz" | "cbr" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "mobi" => 6,
        _ => 7,
    }
}

#[derive(Serialize, Clone, Default)]
pub struct Node {
    pub name: String,
    pub size: u64,
    /// Nombre de fichiers dans tout le sous-arbre
    pub files: u64,
    /// Taille par catégorie
    pub cats: [u64; 8],
    pub dir: bool,
    pub children: Vec<Node>,
    /// Taille des éléments trop petits pour être montrés
    pub rest: u64,
}

pub static CANCEL: AtomicBool = AtomicBool::new(false);

struct Ctx {
    min: u64,
    files: AtomicU64,
    bytes: AtomicU64,
}

fn scan_dir(path: &Path, name: String, ctx: &Ctx) -> Node {
    let mut node = Node { name, dir: true, ..Default::default() };
    if CANCEL.load(Ordering::Relaxed) {
        return node;
    }
    let Ok(rd) = std::fs::read_dir(path) else { return node };
    let mut subdirs = Vec::new();
    let mut big = Vec::new();
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        // Liens et jonctions : pas suivis (sinon des dossiers seraient comptés deux fois)
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            subdirs.push(e.path());
        } else if ft.is_file() {
            // Sous Windows, la taille vient du listage du dossier : pas d'accès en plus
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let name = e.file_name().to_string_lossy().to_string();
            let c = cat(&name);
            node.cats[c] += size;
            node.size += size;
            node.files += 1;
            ctx.files.fetch_add(1, Ordering::Relaxed);
            ctx.bytes.fetch_add(size, Ordering::Relaxed);
            if size >= ctx.min {
                let mut cats = [0; 8];
                cats[c] = size;
                big.push(Node { name, size, files: 1, cats, ..Default::default() });
            }
        }
    }
    let kids: Vec<Node> = subdirs
        .par_iter()
        .map(|p| scan_dir(p, p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), ctx))
        .collect();
    for k in &kids {
        node.size += k.size;
        node.files += k.files;
        for i in 0..8 {
            node.cats[i] += k.cats[i];
        }
    }
    let mut children: Vec<Node> = kids.into_iter().filter(|k| k.size >= ctx.min).chain(big).collect();
    children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    children.truncate(60);
    node.rest = node.size - children.iter().map(|c| c.size).sum::<u64>();
    node.children = children;
    node
}

/// Analyse `root` (ex. « D:\ »). `used` : place occupée sur le disque (fixe le seuil de détail).
/// `progress(fichiers, octets)` est appelé régulièrement.
pub fn analyze(root: &str, used: u64, progress: impl Fn(u64, u64) + Sync) -> Node {
    CANCEL.store(false, Ordering::Relaxed);
    // Assez de détail pour zoomer, sans envoyer des centaines de milliers d'éléments
    let ctx = Ctx { min: (used / 25_000).max(4 << 20), files: AtomicU64::new(0), bytes: AtomicU64::new(0) };
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                progress(ctx.files.load(Ordering::Relaxed), ctx.bytes.load(Ordering::Relaxed));
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
        let n = scan_dir(Path::new(root), root.trim_end_matches('\\').to_string(), &ctx);
        done.store(true, Ordering::Relaxed);
        n
    })
}

#[cfg(test)]
mod tests {
    /// `cargo test disk_map_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn disk_map_live() {
        let t = std::time::Instant::now();
        let root = std::env::var("DISK").unwrap_or("D:\\".into());
        let n = super::analyze(&root, 500 << 30, |_, _| {});
        let json = serde_json::to_string(&n).unwrap();
        println!("{} fichiers, {} Go en {:?}, JSON {} Ko", n.files, n.size >> 30, t.elapsed(), json.len() / 1024);
        for c in n.children.iter().take(12) {
            println!("  {:>8} Mo  {}", c.size >> 20, c.name);
        }
    }
}
