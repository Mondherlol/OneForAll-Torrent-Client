//! Épisodes vus / en cours.
//!
//! Deux sources :
//! - **la lecture lancée depuis OneForAll** (voir `player.rs`) : position et durée exactes, en direct ;
//! - **l'historique de VLC** (`%APPDATA%\vlc\vlc-qt-interface.ini`, section `[RecentsMRL]`), pour ce qui a été
//!   ouvert ailleurs. VLC n'y retient une position qu'entre 5 % et 95 % du fichier : en dessous comme au-dessus
//!   il écrit 0. Un 0 ne veut donc PAS dire « vu » (un épisode à peine lancé donne aussi 0) : on ne s'en sert
//!   que si on sait déjà qu'il avait été regardé en grande partie.
//!
//! Notre propre historique (`watched.json`) garde tout, même quand VLC oublie les plus anciens.

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
    /// Durée de l'épisode (ms), connue quand il a été lu depuis OneForAll
    pub len_ms: i64,
    /// Dernière valeur lue chez VLC : on ne réapplique pas la même (sinon un « non vu » manuel serait écrasé)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlc: Option<i64>,
    /// Qui l'a renseigné : « vlc » (historique), « app » (lecture suivie), « user » (à la main)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub by: String,
}

/// Fin de l'épisode atteinte : 90 % passés, ou moins de 90 s restantes (générique, preview)
pub fn finished(pos_ms: i64, len_ms: i64) -> bool {
    len_ms > 0 && (pos_ms as f64 >= len_ms as f64 * 0.9 || len_ms - pos_ms <= 90_000)
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
pub fn mrl_to_path(mrl: &str) -> Option<String> {
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

/// Ce que l'historique de VLC change pour une entrée (None = rien à en tirer).
fn apply_vlc(e: &mut Watch, ms: i64) -> bool {
    if ms > 0 {
        // Arrêté en route (entre 5 % et 95 %)
        e.watched = e.watched || finished(ms, e.len_ms);
        e.pos_ms = if e.watched { 0 } else { ms };
        true
    } else if e.len_ms > 0 && e.pos_ms * 2 >= e.len_ms {
        // 0 après une position déjà avancée : VLC est allé au bout
        e.watched = true;
        e.pos_ms = 0;
        true
    } else {
        // 0 sans autre indice : à peine commencé OU fini, impossible de savoir → on ne touche à rien
        false
    }
}

impl Watched {
    pub fn new() -> Self {
        let mut map: HashMap<String, Watch> = std::fs::read_to_string(store()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        // Les versions 0.2.13-0.2.14 prenaient le « 0 » de VLC pour « vu » : on annule ces marquages-là
        for w in map.values_mut().filter(|w| w.by.is_empty() && w.vlc == Some(0) && w.watched) {
            w.watched = false;
        }
        Watched { map: Mutex::new(map), mtime: Mutex::new(None) }
    }

    pub fn save(&self) {
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
                changed = true;
                if apply_vlc(e, *ms) {
                    e.by = "vlc".into();
                    // Le premier de la liste est le plus récent
                    e.at = t - i as i64;
                }
            }
        }
        if changed {
            self.save();
        }
    }

    /// Position en direct pendant une lecture lancée depuis OneForAll (ne sauvegarde pas : voir `save`).
    pub fn progress(&self, path: &str, pos_ms: i64, len_ms: i64) {
        let mut map = self.map.lock().unwrap();
        let e = map.entry(key(path)).or_default();
        e.len_ms = len_ms;
        e.at = now();
        e.by = "app".into();
        if finished(pos_ms, len_ms) {
            e.watched = true;
            e.pos_ms = 0;
        } else if pos_ms > 15_000 {
            // Déjà vu et revu depuis le début : il reste « vu », mais on garde la position
            e.pos_ms = pos_ms;
        }
    }

    pub fn get(&self, paths: &[String]) -> HashMap<String, Watch> {
        self.sync();
        let map = self.map.lock().unwrap();
        paths
            .iter()
            .filter_map(|p| {
                let k = key(p);
                // Torrent d'un épisode rangé dans un dossier : la vidéo vue le plus récemment dedans
                let w = map.get(&k).or_else(|| {
                    let dir = format!("{}\\", k.trim_end_matches('\\'));
                    map.iter().filter(|(kk, _)| kk.starts_with(&dir)).map(|(_, w)| w).max_by_key(|w| w.at)
                })?;
                Some((p.clone(), Watch { vlc: None, ..w.clone() }))
            })
            .collect()
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
                e.by = "user".into();
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

    #[test]
    fn zero_is_not_watched() {
        // Lancé puis quitté au bout de 30 s : VLC écrit 0 → PAS vu
        let mut e = Watch::default();
        assert!(!apply_vlc(&mut e, 0));
        assert!(!e.watched);
        // Arrêté à 15 min sur 23 : en cours
        let mut e = Watch { len_ms: 1_380_000, ..Default::default() };
        assert!(apply_vlc(&mut e, 900_000) && !e.watched && e.pos_ms == 900_000);
        // Puis 0 (VLC est allé au-delà de 95 %) : vu
        assert!(apply_vlc(&mut e, 0) && e.watched);
        // Fin d'épisode : 22 min sur 23 = vu
        assert!(finished(1_320_000, 1_380_000) && !finished(600_000, 1_380_000));
    }
}
