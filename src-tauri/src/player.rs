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
use std::sync::{Arc, LazyLock, Mutex};
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

/// Un fichier à lire, la série à laquelle il appartient (pistes préférées) et de quoi l'annoncer
/// dans la fenêtre « Épisode suivant »
#[derive(Clone)]
pub struct Item {
    pub path: PathBuf,
    pub series: Option<String>,
    /// « S1 · É5 » (vide si inconnu)
    pub label: String,
    /// Nom de la série / du film
    pub title: String,
    /// Affiche (URL)
    pub art: Option<String>,
    /// Numéro d'épisode et animé ? (pour trouver le générique sur AniSkip quand le fichier n'a pas de chapitres)
    pub episode: Option<u32>,
    pub anime: bool,
}

// ---------- fenêtre « Épisode suivant » ----------

/// Sans chapitres ni AniSkip : proposée quand il reste moins de 90 s ; compte à rebours de 15 s max
const NEXT_AT_REMAINING_S: i64 = 90;
const NEXT_COUNTDOWN_S: i64 = 15;
/// Ce qui reste après le générique compte comme une « scène » à partir de 15 s (chapitres) / 40 s (AniSkip)
const SCENE_MIN_S: f64 = 15.0;

/// Où en est la proposition « Épisode suivant » pour le fichier en cours
#[derive(PartialEq, Clone, Copy, Debug)]
enum Stage {
    None,
    /// Compte à rebours affiché : à zéro (ou à la fin du fichier) → épisode suivant
    Countdown,
    /// Il reste une scène après le générique : pas de compte à rebours, on attend le choix
    Scene,
    /// On regarde la scène / le générique : proposition à la fin du fichier
    WaitEnd,
    /// Fin du fichier : compte à rebours affiché, VLC est arrêté
    EndCountdown,
    /// Annulé : VLC s'arrêtera à la fin
    Done,
}

/// Opening (début, fin) et générique de fin d'après AniSkip (base communautaire), pour un animé sans chapitres
type Skips = (Option<(f64, f64)>, Option<crate::chapters::Ending>);

fn aniskip(title: &str, ep: u32, len: i64) -> Skips {
    static MAL: LazyLock<Mutex<HashMap<String, Option<i64>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    let res: Option<Skips> = tauri::async_runtime::block_on(async {
        let client = reqwest::Client::builder().user_agent(concat!("OneForAll/", env!("CARGO_PKG_VERSION"))).timeout(Duration::from_secs(8)).build().ok()?;
        let cached = MAL.lock().unwrap().get(title).copied();
        let mal = match cached {
            Some(m) => m?,
            None => {
                // Identifiant MyAnimeList via AniList
                let q = serde_json::json!({ "query": "query($s:String){Media(search:$s,type:ANIME){idMal}}", "variables": { "s": title } });
                let v: serde_json::Value = client.post("https://graphql.anilist.co").json(&q).send().await.ok()?.json().await.ok()?;
                let m = v["data"]["Media"]["idMal"].as_i64();
                MAL.lock().unwrap().insert(title.to_string(), m);
                m?
            }
        };
        let url = format!("https://api.aniskip.com/v2/skip-times/{mal}/{ep}?types[]=op&types[]=mixed-op&types[]=ed&types[]=mixed-ed&episodeLength={len}");
        let v: serde_json::Value = client.get(url).send().await.ok()?.json().await.ok()?;
        let results = v["results"].as_array()?.clone();
        let span = |types: &[&str]| {
            let r = results.iter().find(|r| r["skipType"].as_str().is_some_and(|t| types.contains(&t)))?;
            Some((r["interval"]["startTime"].as_f64()?, r["interval"]["endTime"].as_f64()?))
        };
        let ending = span(&["ed", "mixed-ed"]).map(|(start, end)| {
            let after = (len as f64 - end).max(0.0);
            // Sans chapitres on ne sait pas si c'est un aperçu : seule une longue suite compte comme scène
            crate::chapters::Ending { start, end, after, after_is_preview: after < 40.0 }
        });
        Some((span(&["op", "mixed-op"]), ending))
    });
    res.unwrap_or((None, None))
}

/// Choix fait dans la fenêtre : « next » (lancer maintenant / fin du compte à rebours), « skip » (passer
/// le générique pour voir la scène qui suit) ou « cancel »
static ACTION: Mutex<Option<String>> = Mutex::new(None);

pub fn overlay_action(action: &str) {
    *ACTION.lock().unwrap() = Some(action.to_string());
}

/// Ce qu'affiche la fenêtre (elle le demande au démarrage : commande `overlay_data`)
static OVERLAY_DATA: Mutex<Option<serde_json::Value>> = Mutex::new(None);

pub fn overlay_data() -> Option<serde_json::Value> {
    OVERLAY_DATA.lock().unwrap().clone()
}

/// Journal de la lecture suivie (%APPDATA%\OneForAll\player.log), pour comprendre un souci chez l'utilisateur
pub fn log(msg: &str) {
    use std::io::Write as _;
    let path = crate::config::data_dir().join("player.log");
    // On repart de zéro au-delà de 200 Ko
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 200_000) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "[{t}] {msg}");
    }
}

/// `mode` : « countdown » (compte à rebours), « scene » (une scène suit le générique, `after` s), « end » (fin du fichier)
fn show_overlay(app: &tauri::AppHandle, next: &Item, seconds: i64, mode: &str, after: f64) {
    let data = serde_json::json!({
        "label": next.label,
        "title": next.title,
        "name": next.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        "art": next.art,
        "seconds": seconds,
        "mode": mode,
        "after": after.round() as i64,
    });
    open_overlay(app, data, 440.0, 156.0, &format!("épisode suivant ({mode}) : {}", next.path.display()));
}

/// Petit bouton « Passer l'opening »
fn show_skip_op(app: &tauri::AppHandle) {
    open_overlay(app, serde_json::json!({ "mode": "op" }), 236.0, 64.0, "bouton « passer l'opening »");
}

fn open_overlay(app: &tauri::AppHandle, data: serde_json::Value, w: f64, h: f64, what: &str) {
    hide_overlay(app);
    *OVERLAY_DATA.lock().unwrap() = Some(data);
    let url = tauri::WebviewUrl::App("index.html".into());
    let mut b = tauri::WebviewWindowBuilder::new(app, "overlay", url)
        .title("Épisode suivant")
        .inner_size(w, h)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        // Ne pas prendre le focus : VLC reste en plein écran et garde le clavier
        .focused(false);
    // En bas à droite de l'écran principal, au-dessus de la barre de lecture de VLC
    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let (mw, mh) = (m.size().width as f64 / scale, m.size().height as f64 / scale);
        b = b.position(mw - w - 28.0, mh - h - 120.0);
    }
    match b.build() {
        Ok(_) => log(&format!("fenêtre affichée : {what}")),
        Err(e) => log(&format!("fenêtre impossible ({what}) : {e}")),
    }
}

fn hide_overlay(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.destroy();
    }
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
    /// Lecture arrivée au bout du fichier (« stop state » / « play state: 4 »)
    stopped: bool,
}

fn parse_snapshot(out: &str) -> Snapshot {
    let mut s = Snapshot { uri: RE_INPUT.captures_iter(out).last().map(|c| c[1].to_string()), ..Default::default() };
    // Dernier état annoncé : « ( play state: 3 ) » = lecture, « ( stop state: 5 ) » / « play state: 4 » = fini
    let last_state = out.lines().filter(|l| l.contains("state:") || l.contains("( state ")).last().unwrap_or("");
    s.stopped = last_state.contains("stop state") || last_state.contains("play state: 4") || last_state.contains("state stopped");
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

/// VLC lancé par OneForAll et encore ouvert : on lui envoie les nouveaux épisodes au lieu d'en ouvrir un 2e
struct Session {
    items: Arc<Mutex<Vec<Item>>>,
    /// Commandes à envoyer à VLC (par la boucle de suivi, qui tient la connexion)
    cmds: Arc<Mutex<Vec<String>>>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// « C:\A B\x.mkv » → « file:///C:/A%20B/x.mkv » (pour les commandes add / enqueue de VLC)
fn file_uri(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    let enc: String = s
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~/:".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("file:///{enc}")
}

/// Lance VLC sur ces fichiers (en playlist) et suit la lecture — ou, s'il est déjà ouvert, remplace sa playlist.
pub fn launch(app: tauri::AppHandle, vlc: &Path, items: Vec<Item>) -> Result<(), String> {
    if let Some(sess) = SESSION.lock().unwrap().as_ref() {
        let mut cmds = vec!["clear".to_string(), format!("add {}", file_uri(&items[0].path))];
        cmds.extend(items[1..].iter().map(|i| format!("enqueue {}", file_uri(&i.path))));
        *sess.items.lock().unwrap() = items;
        sess.cmds.lock().unwrap().extend(cmds);
        hide_overlay(&app);
        log("VLC déjà ouvert : nouvelle playlist envoyée");
        return Ok(());
    }
    let port = free_port();
    let mut cmd = std::process::Command::new(vlc);
    cmd.args(["--extraintf", "rc", "--rc-host", &format!("127.0.0.1:{port}"), "--rc-quiet", "--no-one-instance"]);
    // Plein écran, et reprise là où on s'était arrêté sans demander (qt-continue : 2 = toujours)
    cmd.args(["--fullscreen", "--qt-continue=2"]);
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
    // VLC s'arrête à la fin de chaque épisode : c'est la fenêtre « Épisode suivant » (ou la fin de son
    // compte à rebours) qui lance la suite. Toujours actif : la playlist peut être remplacée plus tard.
    cmd.arg("--play-and-stop");
    cmd.args(items.iter().map(|i| &i.path));
    let mut child = cmd.spawn().map_err(|e| format!("VLC ne s'est pas lancé : {e}"))?;
    log(&format!("VLC lancé (port {port}) avec {} fichier(s)", items.len()));
    *ACTION.lock().unwrap() = None;
    let items = Arc::new(Mutex::new(items));
    let cmds = Arc::new(Mutex::new(Vec::new()));
    *SESSION.lock().unwrap() = Some(Session { items: items.clone(), cmds: cmds.clone() });
    std::thread::spawn(move || {
        monitor(&app, port, &items, &cmds);
        *SESSION.lock().unwrap() = None;
        hide_overlay(&app);
        let _ = child.wait();
        log("VLC fermé");
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

fn monitor(app: &tauri::AppHandle, port: u16, shared: &Arc<Mutex<Vec<Item>>>, cmds: &Arc<Mutex<Vec<String>>>) {
    // VLC met un moment à ouvrir son port
    let mut stream = None;
    for _ in 0..60 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            stream = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let Some(mut s) = stream else {
        log("VLC n'a pas ouvert son interface de contrôle : pas de suivi");
        return;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
    let st = app.state::<crate::AppState>();
    let mut cur_uri: Option<String> = None;
    // Pistes déjà remises pour le fichier en cours (on ne retient les choix qu'après)
    let mut applied = false;
    let mut last_save = Instant::now();
    // Fenêtre « Épisode suivant » pour le fichier en cours
    let mut stage = Stage::None;
    // Générique de fin du fichier en cours (chapitres / AniSkip), calculé une fois par fichier
    let mut ending_for: Option<String> = None;
    let mut ending: Option<crate::chapters::Ending> = None;
    // Opening du fichier en cours, et état du bouton « Passer l'opening » (affiché / fermé pour ce fichier)
    let mut opening: Option<(f64, f64)> = None;
    let mut op_shown = false;
    let mut op_dismissed = false;
    // Reprise automatique déjà faite pour ce fichier
    let mut resumed = false;
    loop {
        // Playlist actuelle (remplacée si un autre épisode est lancé depuis l'app)
        let items = shared.lock().unwrap().clone();
        let index: HashMap<String, usize> = items.iter().enumerate().map(|(i, it)| (crate::watch::key(&it.path.display().to_string()), i)).collect();
        let pending: Vec<String> = std::mem::take(&mut *cmds.lock().unwrap());
        if !pending.is_empty() {
            if stage != Stage::None || op_shown {
                hide_overlay(app);
            }
            stage = Stage::None;
            op_shown = false;
            if ask(&mut s, &(pending.join("\n") + "\n")).is_none() {
                break;
            }
        }
        let Some(out) = ask(&mut s, "status\nget_time\nget_length\natrack\nstrack\n") else { break };
        let snap = parse_snapshot(&out);
        if snap.uri.is_some() && snap.uri != cur_uri {
            cur_uri = snap.uri.clone();
            applied = false;
            resumed = false;
            if !matches!(stage, Stage::None | Stage::Done) || op_shown {
                hide_overlay(app);
            }
            stage = Stage::None;
            op_shown = false;
            op_dismissed = false;
        }
        let path = cur_uri.as_deref().and_then(crate::watch::mrl_to_path);
        let pos = path.as_deref().and_then(|p| index.get(&crate::watch::key(p)).copied());
        let (time, len) = (snap.time.unwrap_or(0), snap.length.unwrap_or(0));

        // ----- reprise automatique là où on s'était arrêté -----
        // (VLC le fait déjà avec qt-continue=2 quand il connaît le fichier ; sinon on l'y envoie)
        if let (false, Some(p)) = (resumed, path.as_deref()) {
            if len > 0 && !snap.stopped {
                resumed = true;
                let w = st.watched.get(&[p.to_string()]).remove(p);
                if let Some(w) = w.filter(|w| !w.watched && w.pos_ms > 30_000 && w.pos_ms / 1000 < len - 30) {
                    if time < 10 {
                        let to = (w.pos_ms / 1000 - 3).max(0);
                        log(&format!("reprise à {to} s : {p}"));
                        if ask(&mut s, &format!("seek {to}\n")).is_none() {
                            break;
                        }
                    }
                }
            }
        }
        if let (Some(p), true) = (path.as_deref(), len > 0) {
            st.watched.progress(p, time * 1000, len * 1000);
        }

        // ----- génériques (début, fin) : chapitres du fichier, sinon AniSkip pour un animé -----
        if len > 0 && ending_for != cur_uri {
            ending_for = cur_uri.clone();
            let chaps = path.as_deref().map(|p| crate::chapters::read(Path::new(p))).unwrap_or_default();
            ending = crate::chapters::ending(&chaps, len as f64);
            opening = crate::chapters::opening(&chaps, len as f64);
            if ending.is_none() || opening.is_none() {
                if let Some(it) = pos.map(|i| &items[i]).filter(|it| it.anime) {
                    if let Some(ep) = it.episode {
                        let (op, ed) = aniskip(&it.title, ep, len);
                        opening = opening.or(op);
                        ending = ending.or(ed);
                    }
                }
            }
            log(&format!("opening : {opening:?} — générique de fin : {ending:?}"));
        }

        // ----- bouton « Passer l'opening », affiché pendant l'opening -----
        if let Some((op_start, op_end)) = opening {
            let t = time as f64;
            let inside = !snap.stopped && len > 0 && t >= op_start && t < op_end - 3.0;
            if inside && !op_shown && !op_dismissed && stage == Stage::None {
                show_skip_op(app);
                op_shown = true;
            } else if !inside && op_shown {
                hide_overlay(app);
                op_shown = false;
            }
        }

        // ----- épisode suivant -----
        let next = pos.and_then(|i| items.get(i + 1));
        if let Some(next) = next {
            let start_at = ending.as_ref().map(|e| e.start as i64).unwrap_or(len - NEXT_AT_REMAINING_S);
            let scene = ending.as_ref().filter(|e| e.after >= SCENE_MIN_S && !e.after_is_preview).map(|e| e.after);
            if snap.stopped {
                match stage {
                    // Fin du fichier pendant le compte à rebours : on enchaîne
                    Stage::Countdown => {
                        hide_overlay(app);
                        stage = Stage::None;
                        if ask(&mut s, "next\n").is_none() {
                            break;
                        }
                    }
                    // Scène regardée : maintenant on propose la suite
                    Stage::WaitEnd => {
                        show_overlay(app, next, 10, "end", 0.0);
                        stage = Stage::EndCountdown;
                    }
                    _ => {}
                }
            } else if len > 0 && time > 0 {
                let rem = len - time;
                match stage {
                    Stage::None if time >= start_at && rem >= 3 => {
                        if let Some(after) = scene {
                            show_overlay(app, next, 0, "scene", after);
                            stage = Stage::Scene;
                        } else {
                            show_overlay(app, next, NEXT_COUNTDOWN_S.min(rem - 1), "countdown", 0.0);
                            stage = Stage::Countdown;
                        }
                    }
                    // Retour en arrière avant le générique : on retire la proposition
                    Stage::Countdown | Stage::Scene if time < start_at - 10 => {
                        hide_overlay(app);
                        stage = Stage::None;
                    }
                    _ => {}
                }
            }
        }

        let serie = pos.and_then(|i| items[i].series.clone());
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
        // Attente (1,5 s), en restant réactif aux boutons de la fenêtre « Épisode suivant »
        for _ in 0..6 {
            if !cmds.lock().unwrap().is_empty() {
                break;
            }
            if let Some(action) = ACTION.lock().unwrap().take() {
                hide_overlay(app);
                match action.as_str() {
                    // Passer l'opening : on saute à sa fin
                    "skip_op" => {
                        op_shown = false;
                        op_dismissed = true;
                        if let Some((_, end)) = opening {
                            if ask(&mut s, &format!("seek {}\n", end.ceil() as i64)).is_none() {
                                return st.watched.save();
                            }
                        }
                    }
                    "close_op" => {
                        op_shown = false;
                        op_dismissed = true;
                    }
                    "next" => {
                        stage = Stage::None;
                        if ask(&mut s, "next\n").is_none() {
                            return st.watched.save();
                        }
                    }
                    // Passer le générique pour voir la scène qui suit ; la suite sera proposée à la fin du fichier
                    "skip" => {
                        stage = Stage::WaitEnd;
                        if let Some(e) = &ending {
                            if ask(&mut s, &format!("seek {}\n", e.end.ceil() as i64)).is_none() {
                                return st.watched.save();
                            }
                        }
                    }
                    // Annuler : pendant une scène, on proposera quand même la suite à la fin ; sinon VLC s'arrête
                    _ => stage = if stage == Stage::Scene { Stage::WaitEnd } else { Stage::Done },
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
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
    fn end_of_episode() {
        // Réponses réelles de VLC 3.0.23 (--play-and-stop)
        let playing = "status change: ( play state: 3 )\r\n( state playing )\r\n1375\r\n1380\r\n";
        let ended = "status change: ( play state: 4 ): End\r\nstatus change: ( stop state: 0 )\r\nstatus change: ( stop state: 5 )\r\nstatus: returned 0 (no error)\r\n0\r\n0\r\n";
        let next = "status change: ( new input: file:///C:/x/E02.mkv )\r\nstatus change: ( play state: 3 )\r\n( state playing )\r\n1\r\n1420\r\n";
        assert!(!parse_snapshot(playing).stopped);
        assert!(parse_snapshot(ended).stopped);
        assert!(!parse_snapshot(next).stopped);
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
