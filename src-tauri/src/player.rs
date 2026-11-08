//! Lecture dans VLC, suivie par OneForAll.
//!
//! VLC est lancé avec son interface « RC » (commandes texte sur un port local). Pendant la lecture, on lui
//! demande toutes les 1,5 s : le fichier en cours, la position, la durée, les pistes audio / sous-titres
//! choisies. Ça permet :
//! - de savoir exactement où on s'est arrêté, et si l'épisode a été vu jusqu'au bout ;
//! - de retenir les pistes choisies pour une série (ex. audio japonais + sous-titres français) et de les
//!   remettre toutes seules sur les épisodes suivants.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

/// Piste choisie : désactivée, ou sa langue (« Japonais ») et son rang parmi les pistes du même type
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
pub struct Pick {
    pub off: bool,
    pub lang: String,
    pub nth: usize,
}

/// Pistes préférées d'une série
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
pub struct TrackPref {
    pub audio: Option<Pick>,
    pub sub: Option<Pick>,
}

#[derive(Debug, Clone, PartialEq)]
struct Track {
    id: i64,
    lang: String,
    selected: bool,
}

fn prefs_path() -> PathBuf {
    crate::config::data_dir().join("tracks.json")
}

static PREFS: LazyLock<Mutex<HashMap<String, TrackPref>>> =
    LazyLock::new(|| Mutex::new(std::fs::read_to_string(prefs_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()));

fn save_prefs() {
    if let Ok(json) = serde_json::to_string_pretty(&*PREFS.lock().unwrap()) {
        let _ = std::fs::create_dir_all(crate::config::data_dir());
        let _ = std::fs::write(prefs_path(), json);
    }
}

pub fn prefs_for(series: &str) -> Option<TrackPref> {
    PREFS.lock().unwrap().get(series).cloned()
}

/// Un fichier à lire, et la série à laquelle il appartient (pour les pistes préférées)
pub struct Item {
    pub path: PathBuf,
    pub series: Option<String>,
}

static RE_TRACK: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\|\s*(-?\d+)\s*-\s*(.*?)(\s\*)?\s*$").unwrap());
static RE_LANG: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\[([^\]]+)\]").unwrap());
static RE_INPUT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"new input:\s*(\S+)\s*\)").unwrap());

/// Réponse brute de VLC à « status / get_time / get_length / atrack / strack »
#[derive(Default, Debug)]
struct Snapshot {
    uri: Option<String>,
    time: Option<i64>,
    length: Option<i64>,
    audio: Vec<Track>,
    subs: Vec<Track>,
}

fn parse_snapshot(out: &str) -> Snapshot {
    let mut s = Snapshot { uri: RE_INPUT.captures_iter(out).last().map(|c| c[1].to_string()), ..Default::default() };
    // Nombres seuls sur une ligne : get_time puis get_length (dans l'ordre envoyé)
    let nums: Vec<i64> = out.lines().filter_map(|l| l.trim().parse::<i64>().ok()).collect();
    s.time = nums.first().copied();
    s.length = nums.get(1).copied();
    // Blocs « +----[ Piste audio ] … +----[ end of … ] » : le 1er = audio, le 2e = sous-titres
    let mut blocks: Vec<Vec<Track>> = Vec::new();
    let mut cur: Option<Vec<Track>> = None;
    for line in out.lines().map(str::trim) {
        if line.starts_with("+----[") {
            if line.contains("end of") {
                blocks.extend(cur.take());
            } else {
                cur = Some(Vec::new());
            }
        } else if let (Some(list), Some(c)) = (cur.as_mut(), RE_TRACK.captures(line)) {
            let label = c[2].to_string();
            let lang = RE_LANG.captures_iter(&label).last().map(|l| l[1].trim().to_string()).unwrap_or_default();
            list.push(Track { id: c[1].parse().unwrap_or(-1), lang, selected: c.get(3).is_some() });
        }
    }
    let mut it = blocks.into_iter();
    s.audio = it.next().unwrap_or_default();
    s.subs = it.next().unwrap_or_default();
    s
}

/// Piste actuellement choisie → préférence à retenir
fn current_pick(tracks: &[Track]) -> Option<Pick> {
    let sel = tracks.iter().find(|t| t.selected)?;
    if sel.id < 0 {
        return Some(Pick { off: true, ..Default::default() });
    }
    let real: Vec<&Track> = tracks.iter().filter(|t| t.id >= 0).collect();
    let nth = real.iter().position(|t| t.id == sel.id).map(|i| i + 1).unwrap_or(1);
    Some(Pick { off: false, lang: sel.lang.clone(), nth })
}

/// Préférence → identifiant de piste dans CE fichier (même langue d'abord, sinon même rang)
fn pick_id(tracks: &[Track], p: &Pick) -> Option<i64> {
    if p.off {
        return tracks.iter().any(|t| t.id < 0).then_some(-1);
    }
    let real: Vec<&Track> = tracks.iter().filter(|t| t.id >= 0).collect();
    if !p.lang.is_empty() {
        let same: Vec<&&Track> = real.iter().filter(|t| t.lang.eq_ignore_ascii_case(&p.lang)).collect();
        if let Some(t) = same.iter().find(|t| real.iter().position(|r| r.id == t.id) == Some(p.nth - 1)).or(same.first()) {
            return Some(t.id);
        }
    }
    real.get(p.nth.saturating_sub(1)).map(|t| t.id)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(18094)
}

/// Lance VLC sur ces fichiers (en playlist) et suit la lecture.
pub fn launch(app: tauri::AppHandle, vlc: &Path, items: Vec<Item>) -> Result<(), String> {
    let port = free_port();
    let mut cmd = std::process::Command::new(vlc);
    cmd.args(["--extraintf", "rc", "--rc-host", &format!("127.0.0.1:{port}"), "--rc-quiet", "--no-one-instance"]);
    // Pistes préférées de la série du premier fichier, dès l'ouverture
    if let Some(p) = items.first().and_then(|i| i.series.as_deref()).and_then(prefs_for) {
        if let Some(a) = p.audio.filter(|a| !a.off) {
            cmd.arg(format!("--audio-track={}", a.nth.saturating_sub(1)));
        }
        // (Sous-titres désactivés : remis par le suivi dès l'ouverture, l'option en ligne de commande n'est pas fiable)
        if let Some(s) = p.sub.filter(|s| !s.off) {
            cmd.arg(format!("--sub-track={}", s.nth.saturating_sub(1)));
        }
    }
    cmd.args(items.iter().map(|i| &i.path));
    let mut child = cmd.spawn().map_err(|e| format!("VLC ne s'est pas lancé : {e}"))?;
    let series: HashMap<String, Option<String>> = items.into_iter().map(|i| (crate::watch::key(&i.path.display().to_string()), i.series)).collect();
    std::thread::spawn(move || {
        monitor(&app, port, &series);
        let _ = child.wait();
        let st = app.state::<crate::AppState>();
        st.watched.save();
    });
    Ok(())
}

/// Envoie des commandes et lit la réponse (VLC répond vite : on attend ~0,6 s)
fn ask(s: &mut TcpStream, cmds: &str) -> Option<String> {
    s.write_all(cmds.as_bytes()).ok()?;
    let deadline = Instant::now() + Duration::from_millis(600);
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    while Instant::now() < deadline {
        match s.read(&mut buf) {
            Ok(0) => return None, // VLC fermé
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return None,
        }
    }
    Some(String::from_utf8_lossy(&out).to_string())
}

fn monitor(app: &tauri::AppHandle, port: u16, series: &HashMap<String, Option<String>>) {
    // VLC met un moment à ouvrir son port
    let mut stream = None;
    for _ in 0..60 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            stream = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let Some(mut s) = stream else { return };
    let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
    let st = app.state::<crate::AppState>();
    let mut cur_uri: Option<String> = None;
    // Pistes déjà remises pour le fichier en cours (on ne retient les choix qu'après)
    let mut applied = false;
    let mut last_save = Instant::now();
    loop {
        let Some(out) = ask(&mut s, "status\nget_time\nget_length\natrack\nstrack\n") else { break };
        let snap = parse_snapshot(&out);
        if snap.uri.is_some() && snap.uri != cur_uri {
            cur_uri = snap.uri.clone();
            applied = false;
        }
        let path = cur_uri.as_deref().and_then(crate::watch::mrl_to_path);
        if let (Some(p), Some(t), Some(len)) = (path.as_deref(), snap.time, snap.length) {
            if len > 0 {
                st.watched.progress(p, t * 1000, len * 1000);
            }
        }
        let serie = path.as_deref().and_then(|p| series.get(&crate::watch::key(p)).cloned().flatten());
        if let Some(serie) = serie.filter(|_| !snap.audio.is_empty()) {
            if !applied {
                // Nouveau fichier : on remet les pistes de la série
                if let Some(pref) = prefs_for(&serie) {
                    let mut cmds = String::new();
                    if let Some(id) = pref.audio.as_ref().and_then(|p| pick_id(&snap.audio, p)) {
                        if !snap.audio.iter().any(|t| t.selected && t.id == id) {
                            cmds += &format!("atrack {id}\n");
                        }
                    }
                    if let Some(id) = pref.sub.as_ref().and_then(|p| pick_id(&snap.subs, p)) {
                        if !snap.subs.iter().any(|t| t.selected && t.id == id) {
                            cmds += &format!("strack {id}\n");
                        }
                    }
                    if !cmds.is_empty() && ask(&mut s, &cmds).is_none() {
                        break;
                    }
                }
                applied = true;
            } else {
                // Ensuite, ce qui est choisi devient la préférence de la série
                let now = TrackPref { audio: current_pick(&snap.audio), sub: current_pick(&snap.subs) };
                let mut prefs = PREFS.lock().unwrap();
                if prefs.get(&serie) != Some(&now) && (now.audio.is_some() || now.sub.is_some()) {
                    prefs.insert(serie, now);
                    drop(prefs);
                    save_prefs();
                }
            }
        }
        if last_save.elapsed() > Duration::from_secs(10) {
            st.watched.save();
            last_save = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    st.watched.save();
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "status change: ( new input: file:///C:/Users/x/Dreamland%20S01E01.mkv )\r\n( audio volume: 256 )\r\n( state playing )\r\n4\r\n1380\r\n+----[ Piste audio ]\r\n| -1 - Désactiver\r\n| 1 - Piste 1 - [Japonais] *\r\n| 2 - Piste 2 - [Français]\r\n+----[ end of Piste audio ]\r\n+----[ Piste de sous-titres ]\r\n| -1 - Désactiver\r\n| 3 - Piste 1 - [Français] *\r\n| 4 - Piste 2 - [Allemand]\r\n+----[ end of Piste de sous-titres ]\r\n";

    #[test]
    fn snapshot() {
        let s = parse_snapshot(OUT);
        assert_eq!(s.uri.as_deref(), Some("file:///C:/Users/x/Dreamland%20S01E01.mkv"));
        assert_eq!((s.time, s.length), (Some(4), Some(1380)));
        assert_eq!(s.audio.len(), 3);
        assert_eq!(current_pick(&s.audio), Some(Pick { off: false, lang: "Japonais".into(), nth: 1 }));
        assert_eq!(current_pick(&s.subs), Some(Pick { off: false, lang: "Français".into(), nth: 1 }));
    }


    /// VLC réel, invisible : cargo test vlc_rc_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vlc_rc_live() {
        let file = std::env::var("VLC_TEST_FILE").expect("VLC_TEST_FILE");
        let port = free_port();
        let mut child = std::process::Command::new(r"C:\Program Files\VideoLAN\VLC\vlc.exe")
            .args(["-I", "dummy", "--dummy-quiet", "--extraintf", "rc", "--rc-host", &format!("127.0.0.1:{port}"), "--rc-quiet", "--no-one-instance", "--vout", "dummy", "--aout", "adummy", "--audio-track=1", &file])
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_secs(3));
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let snap = parse_snapshot(&ask(&mut s, "status\nget_time\nget_length\natrack\nstrack\n").unwrap());
        println!("départ : time={:?} len={:?} audio={:?} sous-titres={:?}", snap.time, snap.length, current_pick(&snap.audio), current_pick(&snap.subs));
        // Préférence de la série : japonais + sous-titres français
        let pref = TrackPref { audio: Some(Pick { off: false, lang: "Japonais".into(), nth: 1 }), sub: Some(Pick { off: false, lang: "Français".into(), nth: 1 }) };
        let a = pick_id(&snap.audio, pref.audio.as_ref().unwrap()).unwrap();
        let st = pick_id(&snap.subs, pref.sub.as_ref().unwrap()).unwrap();
        ask(&mut s, &format!("atrack {a}\nstrack {st}\n"));
        let after = parse_snapshot(&ask(&mut s, "status\nget_time\nget_length\natrack\nstrack\n").unwrap());
        println!("après : audio={:?} sous-titres={:?}", current_pick(&after.audio), current_pick(&after.subs));
        let _ = child.kill();
        assert_eq!(current_pick(&after.audio).unwrap().lang, "Japonais");
        assert_eq!(current_pick(&after.subs).unwrap().lang, "Français");
        assert!(after.length.unwrap_or(0) > 0);
    }
    #[test]
    fn picks_follow_language() {
        // Épisode suivant où les pistes sont dans l'autre ordre : on suit la langue, pas le rang
        let audio = vec![
            Track { id: -1, lang: String::new(), selected: false },
            Track { id: 1, lang: "Français".into(), selected: true },
            Track { id: 2, lang: "Japonais".into(), selected: false },
        ];
        assert_eq!(pick_id(&audio, &Pick { off: false, lang: "Japonais".into(), nth: 1 }), Some(2));
        // Langue absente : même rang
        assert_eq!(pick_id(&audio, &Pick { off: false, lang: "Anglais".into(), nth: 2 }), Some(2));
        assert_eq!(pick_id(&audio, &Pick { off: true, ..Default::default() }), Some(-1));
    }
}
