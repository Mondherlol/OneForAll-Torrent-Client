//! Épisodes vus / en cours, d'après l'historique de VLC.
//!
//! VLC garde ses 30 derniers fichiers ouverts dans `%APPDATA%\vlc\vlc-qt-interface.ini`
//! (section `[RecentsMRL]` : `list=` les fichiers, `times=` la position d'arrêt en ms ;
//! 0 = lu jusqu'au bout, ou à peine commencé). On en garde notre propre historique
//! (`watched.json`) pour ne rien perdre quand VLC oublie les plus anciens.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Watch {
    /// Position d'arrêt (ms) si l'épisode est en cours
    pub pos_ms: i64,
    pub watched: bool,
    /// Dernière fois vu (secondes Unix), pour trouver « où j'en suis »
    pub at: i64,
    /// Dernière valeur lue chez VLC : on ne réapplique pas la même (sinon un « non vu » manuel serait écrasé)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlc: Option<i64>,
}

pub struct Watched {
    map: Mutex<HashMap<String, Watch>>,
    mtime: Mutex<Option<SystemTime>>,
}

fn now() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn store() -> PathBuf {
    crate::config::data_dir().join("watched.json")
}

fn vlc_ini() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("vlc").join("vlc-qt-interface.ini")
}

/// Clé commune aux chemins Windows et aux adresses VLC : « d:\series\x.mkv »
pub fn key(path: &str) -> String {
    path.trim().replace('/', "\\").to_lowercase()
}

/// Liste QSettings : « a, b, "c, avec virgule" »
fn split_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// « file:///D:/Series/A%20B.mkv » → « D:\Series\A B.mkv »
fn mrl_to_path(mrl: &str) -> Option<String> {
    let rest = mrl.strip_prefix("file:///")?;
    Some(crate::search::pct_decode(rest).replace('/', "\\"))
}

/// (chemin, position en ms), du plus récent au plus ancien
pub fn parse_ini(ini: &str) -> Vec<(String, i64)> {
    let mut list = Vec::new();
    let mut times = Vec::new();
    let mut in_section = false;
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[RecentsMRL]";
        } else if in_section {
            if let Some(v) = line.strip_prefix("list=") {
                list = split_list(v);
            } else if let Some(v) = line.strip_prefix("times=") {
                times = split_list(v).iter().map(|t| t.parse::<i64>().unwrap_or(0)).collect();
            }
        }
    }
    list.iter().enumerate().filter_map(|(i, m)| Some((mrl_to_path(m)?, times.get(i).copied().unwrap_or(0)))).collect()
}

impl Watched {
    pub fn new() -> Self {
        let map = std::fs::read_to_string(store()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Watched { map: Mutex::new(map), mtime: Mutex::new(None) }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string(&*self.map.lock().unwrap()) {
            let _ = std::fs::create_dir_all(crate::config::data_dir());
            let _ = std::fs::write(store(), json);
        }
    }

    /// Relit l'historique de VLC s'il a changé.
    pub fn sync(&self) {
        let path = vlc_ini();
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else { return };
        if *self.mtime.lock().unwrap() == Some(mtime) {
            return;
        }
        *self.mtime.lock().unwrap() = Some(mtime);
        let Ok(ini) = std::fs::read_to_string(&path) else { return };
        let entries = parse_ini(&ini);
        let t = now();
        let mut changed = false;
        {
            let mut map = self.map.lock().unwrap();
            for (i, (p, ms)) in entries.iter().enumerate() {
                let e = map.entry(key(p)).or_default();
                if e.vlc == Some(*ms) {
                    continue;
                }
                e.vlc = Some(*ms);
                e.pos_ms = (*ms).max(0);
                e.watched = *ms <= 0;
                // Le premier de la liste est le plus récent
                e.at = t - i as i64;
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    pub fn get(&self, paths: &[String]) -> HashMap<String, Watch> {
        self.sync();
        let map = self.map.lock().unwrap();
        paths.iter().filter_map(|p| map.get(&key(p)).map(|w| (p.clone(), Watch { vlc: None, ..w.clone() }))).collect()
    }

    /// Marquage à la main (vu / pas vu)
    pub fn set(&self, paths: &[String], watched: bool) {
        {
            let mut map = self.map.lock().unwrap();
            let t = now();
            for p in paths {
                let e = map.entry(key(p)).or_default();
                e.watched = watched;
                e.pos_ms = 0;
                if watched {
                    e.at = t;
                }
            }
        }
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lit le vrai historique VLC (lecture seule) : cargo test vlc_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vlc_real() {
        let ini = std::fs::read_to_string(vlc_ini()).unwrap();
        let e = parse_ini(&ini);
        for (p, ms) in e.iter().take(6) {
            println!("{ms:>8} ms  {p}");
        }
        println!("{} entrées, {} en cours", e.len(), e.iter().filter(|x| x.1 > 0).count());
        assert!(!e.is_empty());
    }

    #[test]
    fn vlc_history() {
        let ini = "[General]\nx=1\n[RecentsMRL]\nlist=file:///D:/Series/A%20B%20-%2001.mkv, \"file:///C:/x,y.mkv\", http://stream\ntimes=0, 1137634, 5\n[Other]\nlist=nope";
        let e = parse_ini(ini);
        assert_eq!(e, vec![(r"D:\Series\A B - 01.mkv".to_string(), 0), (r"C:\x,y.mkv".to_string(), 1137634)]);
        assert_eq!(key(r"D:/Series/A.mkv"), key(r"d:\series\a.MKV"));
    }
}
