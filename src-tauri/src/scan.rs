//! Recherche des fichiers .torrent sur les disques et calcul de leur infohash,
//! pour savoir lesquels ne sont plus utilisÃ©s par qBittorrent.

use serde::Serialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;

#[derive(Serialize, Clone, Debug)]
pub struct TorrentFileInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: i64,
    /// sha1 de Â« info Â» (v1) ; sha256 complet pour v2
    pub v1: Option<String>,
    pub v2: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Disk {
    pub mount: String,
    pub name: String,
    pub total: u64,
    pub free: u64,
    pub removable: bool,
}

pub fn disks() -> Vec<Disk> {
    let Ok(list) = sysinfo::Disks::new_with_refreshed_list() else { return Vec::new() };
    let mut out: Vec<Disk> = list
        .iter()
        .map(|d| Disk {
            mount: d.mount_point().to_string_lossy().to_string(),
            name: d.name().to_string_lossy().to_string(),
            total: d.total_space(),
            free: d.available_space(),
            removable: d.is_removable(),
        })
        .collect();
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out.dedup_by(|a, b| a.mount == b.mount);
    out
}

/// Dossiers jamais parcourus : systÃ¨me, programmes, et les .torrent internes de qBittorrent.
const SKIP: &[&str] = &[
    "windows",
    "$recycle.bin",
    "system volume information",
    "program files",
    "program files (x86)",
    "programdata",
    "appdata",
    "node_modules",
    ".git",
    "bt_backup",
    "oneforall-target",
    "$winreagent",
    "recovery",
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parcourt tous les disques fixes. `progress` reÃ§oit le nombre de dossiers vus.
pub fn find_torrents(counter: &AtomicUsize) -> Vec<TorrentFileInfo> {
    let roots: Vec<PathBuf> = disks().into_iter().filter(|d| !d.removable).map(|d| PathBuf::from(d.mount)).collect();
    let mut out = Vec::new();
    for root in roots {
        let walker = WalkDir::new(&root).follow_links(false).into_iter().filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let n = e.file_name().to_string_lossy().to_lowercase();
            !SKIP.contains(&n.as_str())
        });
        for entry in walker.flatten() {
            if entry.file_type().is_dir() {
                counter.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let p = entry.path();
            let is_torrent = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("torrent"));
            if !is_torrent {
                continue;
            }
            if let Some(info) = inspect(p) {
                out.push(info);
            }
        }
    }
    out
}

fn inspect(p: &Path) -> Option<TorrentFileInfo> {
    let meta = std::fs::metadata(p).ok()?;
    if meta.len() > 50 * 1024 * 1024 {
        return None;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let data = std::fs::read(p).ok()?;
    let (v1, v2, name) = match info_span(&data) {
        Some((s, e)) => {
            let info = &data[s..e];
            let is_v2 = dict_get(info, b"meta version").is_some();
            let has_pieces = dict_get(info, b"pieces").is_some();
            let v1 = has_pieces.then(|| hex(&Sha1::digest(info)));
            let v2 = is_v2.then(|| hex(&Sha256::digest(info)));
            let name = dict_get(info, b"name").and_then(|(a, b)| parse_str(&info[a..b])).map(|s| String::from_utf8_lossy(s).to_string());
            (v1, v2, name)
        }
        None => (None, None, None),
    };
    Some(TorrentFileInfo {
        path: p.to_string_lossy().to_string(),
        name: name.unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
        size: meta.len(),
        modified,
        v1,
        v2,
    })
}

/// Identifiant (infohash) d'un fichier .torrent, tel que qBittorrent l'utilise.
pub fn infohash(data: &[u8]) -> Option<String> {
    let (s, e) = info_span(data)?;
    let info = &data[s..e];
    if dict_get(info, b"pieces").is_some() {
        Some(hex(&Sha1::digest(info)))
    } else {
        // torrent v2 uniquement : sha256 tronqué à 40 caractères
        Some(hex(&Sha256::digest(info))[..40].to_string())
    }
}

/// Éléments d'une liste bencodée (plages de chaque valeur).
fn list_items(d: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if d.first() != Some(&b'l') {
        return out;
    }
    let mut i = 1;
    while d.get(i).is_some_and(|c| *c != b'e') {
        let Some(end) = skip(d, i) else { break };
        out.push((i, end));
        i = end;
    }
    out
}

fn parse_int(d: &[u8]) -> Option<i64> {
    std::str::from_utf8(d.strip_prefix(b"i")?.strip_suffix(b"e")?).ok()?.parse().ok()
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct TorrentEntry {
    pub path: String,
    pub size: i64,
}

/// Nom et liste des fichiers d'un .torrent, sans l'ajouter à qBittorrent.
pub fn torrent_files(data: &[u8]) -> Option<(String, Vec<TorrentEntry>)> {
    let (s, e) = info_span(data)?;
    let info = &data[s..e];
    let str_of = |r: (usize, usize), d: &[u8]| parse_str(&d[r.0..r.1]).map(|b| String::from_utf8_lossy(b).to_string());
    let name = dict_get(info, b"name").and_then(|r| str_of(r, info)).unwrap_or_default();
    let mut files = Vec::new();
    if let Some((a, b)) = dict_get(info, b"files") {
        let list = &info[a..b];
        for (fa, fb) in list_items(list) {
            let f = &list[fa..fb];
            let size = dict_get(f, b"length").and_then(|(x, y)| parse_int(&f[x..y])).unwrap_or(0);
            let path = dict_get(f, b"path")
                .map(|(x, y)| {
                    let p = &f[x..y];
                    list_items(p).into_iter().filter_map(|r| str_of(r, p)).collect::<Vec<_>>().join("/")
                })
                .unwrap_or_default();
            files.push(TorrentEntry { path, size });
        }
    } else {
        let size = dict_get(info, b"length").and_then(|(x, y)| parse_int(&info[x..y])).unwrap_or(0);
        files.push(TorrentEntry { path: name.clone(), size });
    }
    Some((name, files))
}

/// Identifiant d'un lien magnet (hexadécimal ou base32).
pub fn magnet_hash(uri: &str) -> Option<String> {
    let lower = uri.to_lowercase();
    let start = lower.find("xt=urn:btih:")? + "xt=urn:btih:".len();
    let id: String = uri[start..].chars().take_while(|c| *c != '&').collect();
    match id.len() {
        40 => Some(id.to_lowercase()),
        32 => {
            // base32 (RFC 4648) → hexadécimal
            let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
            let mut bits: u64 = 0;
            let mut nbits = 0;
            let mut out = Vec::new();
            for c in id.to_uppercase().bytes() {
                let v = alphabet.iter().position(|a| *a == c)? as u64;
                bits = (bits << 5) | v;
                nbits += 5;
                if nbits >= 8 {
                    nbits -= 8;
                    out.push((bits >> nbits) as u8);
                    bits &= (1 << nbits) - 1;
                }
            }
            Some(hex(&out))
        }
        _ => None,
    }
}

// ---------- mini-lecteur bencode ----------

/// Renvoie la fin (exclue) de la valeur bencodÃ©e qui commence Ã  `i`.
fn skip(d: &[u8], i: usize) -> Option<usize> {
    match *d.get(i)? {
        b'i' => Some(i + d[i..].iter().position(|&c| c == b'e')? + 1),
        b'l' | b'd' => {
            let mut j = i + 1;
            while *d.get(j)? != b'e' {
                j = skip(d, j)?;
            }
            Some(j + 1)
        }
        b'0'..=b'9' => {
            let colon = i + d[i..].iter().position(|&c| c == b':')?;
            let len: usize = std::str::from_utf8(&d[i..colon]).ok()?.parse().ok()?;
            let end = colon + 1 + len;
            (end <= d.len()).then_some(end)
        }
        _ => None,
    }
}

fn parse_str(d: &[u8]) -> Option<&[u8]> {
    let colon = d.iter().position(|&c| c == b':')?;
    let len: usize = std::str::from_utf8(&d[..colon]).ok()?.parse().ok()?;
    d.get(colon + 1..colon + 1 + len)
}

/// Dans un dictionnaire bencodÃ©, renvoie la plage de la valeur associÃ©e Ã  `key`.
fn dict_get(d: &[u8], key: &[u8]) -> Option<(usize, usize)> {
    if d.first() != Some(&b'd') {
        return None;
    }
    let mut i = 1;
    while *d.get(i)? != b'e' {
        let kend = skip(d, i)?;
        let k = parse_str(&d[i..kend])?;
        let vend = skip(d, kend)?;
        if k == key {
            return Some((kend, vend));
        }
        i = vend;
    }
    None
}

fn info_span(d: &[u8]) -> Option<(usize, usize)> {
    dict_get(d, b"info")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_list() {
        let t = b"d4:infod5:filesld6:lengthi10e4:pathl2:S15:e.mkveed6:lengthi5e4:pathl5:a.nfoeee4:name4:Show12:piece lengthi1e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";
        let (name, files) = torrent_files(t).unwrap();
        assert_eq!(name, "Show");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "S1/e.mkv");
        assert_eq!(files[0].size, 10);
    }

    #[test]
    fn magnets() {
        assert_eq!(
            magnet_hash("magnet:?xt=urn:btih:23269595DFD15202E3144C586AF05498D60F71CC&dn=x").as_deref(),
            Some("23269595dfd15202e3144c586af05498d60f71cc")
        );
        // même hash en base32
        assert_eq!(
            magnet_hash("magnet:?xt=urn:btih:EMTJLFO72FJAFYYUJRMGV4CUTDLA64OM").as_deref(),
            Some("23269595dfd15202e3144c586af05498d60f71cc")
        );
    }

    #[test]
    fn bencode_info_hash() {
        let t = b"d8:announce3:abc4:infod6:lengthi5e4:name5:hello12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";
        let (s, e) = info_span(t).unwrap();
        let info = &t[s..e];
        assert!(info.starts_with(b"d6:length"));
        assert!(info.ends_with(b"aaaae"));
        let (a, b) = dict_get(info, b"name").unwrap();
        assert_eq!(parse_str(&info[a..b]).unwrap(), b"hello");
    }
}
