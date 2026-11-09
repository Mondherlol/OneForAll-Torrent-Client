//! ROM déjà présentes sur les disques (téléchargées avant OneForAll, copiées…) :
//! recherchées sur tous les disques, gardées dans roms.json, affichées dans l'onglet Jeux.

use crate::emu;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LocalRom {
    pub path: String,
    pub title: String,
    pub platform: String,
    pub emu: String,
    pub size: u64,
    /// Date du fichier (secondes Unix)
    pub added_on: i64,
    pub art_key: String,
}

/// Dossiers où il n'y a pas de jeux à soi (système, programmes, caches, projets de code)
const SKIP: &[&str] = &[
    "windows", "program files", "program files (x86)", "programdata", "appdata", "$recycle.bin", "system volume information",
    "$windows.~bt", "$windows.~ws", "recovery", "perflogs", "msocache", "node_modules", ".git", ".cargo", ".rustup", ".npm",
    "steamapps", "windowsapps", "onedrivetemp", "$sysreset", "config.msi", "intel", "amd", "nvidia", "drivers",
    ".android", ".gradle", ".vscode", ".nuget", "emulators", "atmosphere", "uploads",
];

fn file() -> PathBuf {
    crate::config::data_dir().join("roms.json")
}

/// ROM trouvées au dernier scan (sans celles qui ont disparu)
pub fn library() -> Vec<LocalRom> {
    let list: Vec<LocalRom> = std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.into_iter().filter(|r| Path::new(&r.path).exists()).collect()
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Fichiers candidats (extension de ROM, d'image disque ou d'archive), dossiers système exclus
fn candidates(root: &Path, seen: &AtomicU64) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let n = e.file_name().to_string_lossy().to_lowercase();
            !SKIP.contains(&n.as_str()) && !n.starts_with('$')
        })
        .flatten()
        .filter(|e| e.file_type().is_file())
        .inspect(|_| {
            seen.fetch_add(1, Ordering::Relaxed);
        })
        .map(|e| e.into_path())
        .filter(|p| {
            let x = ext(p);
            // .nro / .prx : programmes maison et modules, pas des jeux
            (emu::is_rom(p) && !matches!(x.as_str(), "nro" | "prx")) || matches!(x.as_str(), "zip" | "7z")
        })
        .collect()
}

/// Les cartouches Game Boy / GBA / DS ont un en-tête reconnaissable : évite les fichiers qui
/// ont juste la même extension (caches Unity en .gbc…)
fn cartridge_ok(p: &Path) -> bool {
    use std::io::Read;
    let (at, want): (usize, &[u8]) = match ext(p).as_str() {
        "gb" | "gbc" | "sgb" => (0x104, &[0xCE, 0xED, 0x66, 0x66]),
        "gba" => (0xB2, &[0x96]),
        "nds" | "dsi" | "srl" => (0x15C, &[0x56, 0xCF]),
        _ => return true,
    };
    let mut buf = vec![0u8; at + want.len()];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut buf)).is_ok() && &buf[at..] == want
}

/// « 0905 - Danny Phantom (Europe) » → « Danny Phantom » ; « [DS].Mario.Et.Sonic.[EU] » → « Mario Et Sonic »
fn title_of(stem: &str) -> String {
    static NUM: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^\d{3,5}\s*-\s*").unwrap());
    let mut s = stem.replace('_', " ");
    if !s.contains(' ') {
        s = s.replace(['.', '-'], " ");
    }
    let s = crate::strip_tags(&s);
    let s = NUM.replace(&s, "");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    // « Legend of Zelda, The - The Minish Cap » → « The Legend of Zelda - The Minish Cap »
    let (head, tail) = s.split_once(" - ").map(|(h, t)| (h.to_string(), format!(" - {t}"))).unwrap_or((s.clone(), String::new()));
    for art in [", The", ", A", ", An", ", Les", ", Le", ", La", ", L'"] {
        if let Some(h) = head.strip_suffix(art) {
            return format!("{} {h}{tail}", &art[2..]);
        }
    }
    s
}

/// Cherche les ROM sur tous les disques. `progress(fichiers vus, ROM trouvées)`.
pub fn scan(progress: impl Fn(u64, u64) + Sync) -> Vec<LocalRom> {
    let seen = AtomicU64::new(0);
    let found = AtomicU64::new(0);
    let roots: Vec<PathBuf> = crate::scan::disks().into_iter().map(|d| PathBuf::from(d.mount)).collect();
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                progress(seen.load(Ordering::Relaxed), found.load(Ordering::Relaxed));
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        let mut files: Vec<PathBuf> = roots.par_iter().flat_map(|r| candidates(r, &seen)).collect();
        // Avec un .cue, les .bin du même dossier sont ses pistes : seul le .cue compte
        let cue_dirs: std::collections::HashSet<PathBuf> = files.iter().filter(|p| ext(p) == "cue").filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
        files.retain(|p| !(ext(p) == "bin" && p.parent().is_some_and(|d| cue_dirs.contains(d))));
        files = files
            .par_iter()
            .filter(|p| {
                let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                // Archives : entre 16 Ko et 16 Go (au-delà ce n'est pas une ROM) ; .bin/.img : au moins 1 Mo
                match ext(p).as_str() {
                    "zip" | "7z" => (16 << 10..16 << 30).contains(&size),
                    "bin" | "img" => size >= 1 << 20,
                    _ => size > 0,
                }
            })
            .cloned()
            .collect();
        let roms: Vec<LocalRom> = files
            .par_iter()
            .filter_map(|p| {
                if !cartridge_ok(p) {
                    return None;
                }
                let m = emu::detect(p)?;
                let meta = std::fs::metadata(p).ok()?;
                // Un jeu Switch pèse plus de 50 Mo (les petits .nsp / .zip sont des outils et des packs)
                if m.id == "ryujinx" && meta.len() < 50 << 20 {
                    return None;
                }
                found.fetch_add(1, Ordering::Relaxed);
                let title = title_of(&p.file_stem()?.to_string_lossy());
                Some(LocalRom {
                    path: p.display().to_string(),
                    art_key: format!("game:{}", crate::media::normalize(&title)),
                    title,
                    platform: emu::platform_of(m.id).to_string(),
                    emu: m.id.to_string(),
                    size: meta.len(),
                    // Arrivée sur le PC : la plus récente des dates de création et de modification
                    // (une ROM extraite garde souvent la date d'origine du jeu, « il y a 30 ans »)
                    added_on: [meta.created().ok(), meta.modified().ok()]
                        .into_iter()
                        .flatten()
                        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .max()
                        .unwrap_or(0),
                })
            })
            .collect();
        done.store(true, Ordering::Relaxed);
        let mut roms = roms;
        // Même jeu en .zip et extrait à côté : on garde la ROM extraite (elle se lance directement)
        let key = |r: &LocalRom| r.title.to_lowercase().replace('&', "and").chars().filter(|c| c.is_alphanumeric()).collect::<String>();
        roms.sort_by_key(|r| (key(r), r.platform.clone(), matches!(ext(Path::new(&r.path)).as_str(), "zip" | "7z")));
        roms.dedup_by(|b, a| key(a) == key(b) && a.platform == b.platform);
        roms.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        if let Ok(b) = serde_json::to_vec_pretty(&roms) {
            let _ = std::fs::write(file(), b);
        }
        progress(seen.load(Ordering::Relaxed), roms.len() as u64);
        roms
    })
}

#[cfg(test)]
mod tests {
    /// `cargo test rom_scan_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn rom_scan_live() {
        let t = std::time::Instant::now();
        let roms = super::scan(|_, _| {});
        println!("{} ROM en {:?}", roms.len(), t.elapsed());
        for r in &roms {
            println!("{} | {} | {}", r.title, r.platform, r.path);
        }
    }
}
