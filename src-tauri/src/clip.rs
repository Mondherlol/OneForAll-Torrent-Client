//! Studio d'extraits : lire n'importe quelle vidéo locale (x265 10 bits, Blu-ray, MKV multi-pistes, sous-titres
//! texte ou image, HDR) et en exporter des passages en MP4 partageable, piste audio et sous-titres au choix.
//!
//! L'aperçu : un petit serveur local (127.0.0.1) envoie la vidéo convertie à la volée par ffmpeg (H.264 + AAC,
//! MP4 fragmenté) à partir de l'instant demandé, avec la piste audio choisie et les sous-titres incrustés.
//! Le navigateur intégré ne sait lire ni le x265, ni le DTS, ni les sous-titres des MKV : c'est la seule façon fiable.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, OnceLock};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Serialize, Clone, Default, Debug)]
pub struct Track {
    /// Rang parmi les pistes de ce type (0:a:N / 0:s:N)
    pub n: usize,
    pub codec: String,
    pub lang: String,
    pub title: String,
    pub channels: i64,
    pub default: bool,
    pub forced: bool,
    /// Sous-titres en images (Blu-ray PGS, DVD) : incrustés par superposition
    pub image: bool,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Source {
    pub id: String,
    pub path: String,
    pub name: String,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub vcodec: String,
    pub hdr: bool,
    pub audio: Vec<Track>,
    pub subs: Vec<Track>,
    /// Adresse de l'aperçu (ajouter ?t=…&a=…&s=…)
    pub stream: String,
}

/// Un passage à exporter
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Seg {
    pub start: f64,
    pub end: f64,
    /// Piste audio (rang), None = muet
    pub audio: Option<usize>,
    /// Sous-titres (rang), None = aucun
    pub sub: Option<usize>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Spec {
    pub path: String,
    pub segs: Vec<Seg>,
    /// Hauteur maximale de sortie (1080, 720…)
    pub height: i64,
}

static SOURCES: LazyLock<Mutex<HashMap<String, Source>>> = LazyLock::new(Default::default);
static FF: OnceLock<PathBuf> = OnceLock::new();
static PORT: OnceLock<u16> = OnceLock::new();
static ZSCALE: OnceLock<bool> = OnceLock::new();

fn ff_cmd(exe: &str) -> Command {
    let dir = FF.get().cloned().unwrap_or_default();
    let mut c = Command::new(dir.join(exe));
    #[cfg(windows)]
    c.creation_flags(0x0800_0000);
    c
}

/// Le ffmpeg dispose-t-il du filtre zscale (conversion HDR → SDR propre) ?
fn has_zscale() -> bool {
    *ZSCALE.get_or_init(|| {
        ff_cmd("ffmpeg.exe").args(["-hide_banner", "-filters"]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains(" zscale ")).unwrap_or(false)
    })
}

const IMAGE_SUBS: &[&str] = &["hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle", "xsub"];
const TEXT_SUBS: &[&str] = &["subrip", "ass", "ssa", "webvtt", "mov_text", "text"];

pub fn open(ffdir: &Path, path: &str) -> Result<Source, String> {
    let _ = FF.set(ffdir.to_path_buf());
    // Dossier (torrent d'un film / d'un épisode) : sa plus grosse vidéo
    let found;
    let p = if Path::new(path).is_dir() {
        found = walkdir::WalkDir::new(path)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                [".mkv", ".mp4", ".avi", ".m4v", ".mov", ".webm", ".ts", ".m2ts", ".wmv"].iter().any(|x| n.ends_with(x))
            })
            .max_by_key(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .map(|e| e.path().to_path_buf())
            .ok_or("Aucune vidéo dans ce dossier")?;
        found.as_path()
    } else {
        Path::new(path)
    };
    let path = &p.display().to_string();
    if !p.is_file() {
        return Err("Ce fichier n'existe pas (encore).".into());
    }
    let out = ff_cmd("ffprobe.exe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(p)
        .output()
        .map_err(|e| format!("ffprobe introuvable : {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| "Vidéo illisible".to_string())?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64() != Some(1))
        .ok_or("Pas d'image dans ce fichier")?;
    let tag = |s: &serde_json::Value, k: &str| s["tags"][k].as_str().or(s["tags"][k.to_uppercase()].as_str()).unwrap_or("").to_string();
    let flag = |s: &serde_json::Value, k: &str| s["disposition"][k].as_i64() == Some(1);
    let mut audio = Vec::new();
    let mut subs = Vec::new();
    let (mut na, mut ns) = (0, 0);
    for s in &streams {
        let codec = s["codec_name"].as_str().unwrap_or("").to_string();
        match s["codec_type"].as_str() {
            Some("audio") => {
                audio.push(Track {
                    n: na,
                    lang: tag(s, "language"),
                    title: tag(s, "title"),
                    channels: s["channels"].as_i64().unwrap_or(0),
                    default: flag(s, "default"),
                    codec,
                    ..Default::default()
                });
                na += 1;
            }
            Some("subtitle") => {
                let image = IMAGE_SUBS.contains(&codec.as_str());
                if image || TEXT_SUBS.contains(&codec.as_str()) {
                    subs.push(Track {
                        n: ns,
                        lang: tag(s, "language"),
                        title: tag(s, "title"),
                        default: flag(s, "default"),
                        forced: flag(s, "forced") || tag(s, "title").to_lowercase().contains("forc"),
                        image,
                        codec,
                        ..Default::default()
                    });
                }
                ns += 1;
            }
            _ => {}
        }
    }
    let duration = v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let transfer = video["color_transfer"].as_str().unwrap_or("");
    let id = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut h);
        format!("{:x}", h.finish())
    };
    let src = Source {
        stream: format!("http://127.0.0.1:{}/v/{id}", server()),
        id: id.clone(),
        path: path.to_string(),
        name: p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        duration,
        width: video["width"].as_i64().unwrap_or(0),
        height: video["height"].as_i64().unwrap_or(0),
        vcodec: video["codec_name"].as_str().unwrap_or("").to_string(),
        hdr: transfer == "smpte2084" || transfer == "arib-std-b67",
        audio,
        subs,
    };
    SOURCES.lock().unwrap().insert(id, src.clone());
    let s2 = src.clone();
    std::thread::spawn(move || prepare(&s2));
    Ok(src)
}

fn cache_dir(id: &str) -> PathBuf {
    crate::config::data_dir().join("clipcache").join(id)
}

/// Sous-titres texte extraits (.ass) et polices du MKV : le filtre « subtitles » relirait sinon tout le fichier
/// (plusieurs secondes sur un épisode) à chaque saut dans la vidéo.
fn prepare(src: &Source) {
    let dir = cache_dir(&src.id);
    if dir.join("ready").is_file() {
        return;
    }
    let _ = std::fs::create_dir_all(dir.join("fonts"));
    let text: Vec<&Track> = src.subs.iter().filter(|s| !s.image).collect();
    if !text.is_empty() {
        let mut c = ff_cmd("ffmpeg.exe");
        c.args(["-v", "error", "-y", "-nostdin", "-i"]).arg(&src.path);
        for s in &text {
            c.args(["-map", &format!("0:s:{}", s.n), "-c:s", "ass"]).arg(dir.join(format!("s{}.ass", s.n)));
        }
        let _ = c.stdout(Stdio::null()).stderr(Stdio::null()).status();
        // Polices jointes (sous-titres stylés des animés)
        let _ = ff_cmd("ffmpeg.exe")
            .args(["-v", "quiet", "-nostdin", "-dump_attachment:t", "", "-i"])
            .arg(&src.path)
            .current_dir(dir.join("fonts"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = std::fs::write(dir.join("ready"), "");
}

/// Fichier .ass prêt pour cette piste (attend un peu la préparation si elle est en cours)
fn cached_sub(src: &Source, n: usize) -> Option<PathBuf> {
    let dir = cache_dir(&src.id);
    for _ in 0..80 {
        if dir.join("ready").is_file() {
            return Some(dir.join(format!("s{n}.ass"))).filter(|p| p.is_file());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    None
}

/// Échappe un nom de fichier pour le filtre « subtitles » (deux niveaux : option, puis graphe de filtres)
fn esc_filter(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if matches!(c, '\\' | '\'' | ':') {
            o.push('\\');
        }
        o.push(c);
    }
    let mut g = String::new();
    for c in o.chars() {
        if matches!(c, '\\' | '\'' | '[' | ']' | ',' | ';') {
            g.push('\\');
        }
        g.push(c);
    }
    g
}

/// Arguments ffmpeg (après -i) qui produisent l'image : HDR → SDR, sous-titres incrustés, redimensionnement.
/// `t` = instant de départ (entrée déjà positionnée avec -ss) : les sous-titres texte en ont besoin pour rester calés.
fn video_args(src: &Source, sub: Option<usize>, t: f64, height: i64) -> Vec<String> {
    let tm = if src.hdr && has_zscale() {
        "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv"
    } else {
        ""
    };
    let scale = if height > 0 && height < src.height { format!("scale=-2:{height}:flags=bicubic,") } else { String::new() };
    let track = sub.and_then(|n| src.subs.iter().find(|s| s.n == n));
    match track {
        Some(s) if s.image => {
            let base = if tm.is_empty() { "null".to_string() } else { tm.to_string() };
            let graph = format!("[0:v:0]{base}[b];[b][0:s:{}]overlay=(W-w)/2:(H-h)/2:eof_action=pass,{scale}format=yuv420p[v]", s.n);
            vec!["-filter_complex".into(), graph, "-map".into(), "[v]".into()]
        }
        Some(s) => {
            let tm = if tm.is_empty() { String::new() } else { format!("{tm},") };
            let subs = match cached_sub(src, s.n) {
                Some(f) => {
                    let fonts = f.with_file_name("fonts");
                    format!("subtitles=filename={}:fontsdir={}", esc_filter(&f.display().to_string()), esc_filter(&fonts.display().to_string()))
                }
                // Repli : lecture directe dans la vidéo (plus lent)
                None => {
                    let name = Path::new(&src.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    format!("subtitles=filename={}:si={}", esc_filter(&name), s.n)
                }
            };
            let chain = format!("setpts=PTS+{t:.3}/TB,{tm}{subs},setpts=PTS-STARTPTS,{scale}format=yuv420p");
            vec!["-vf".into(), chain, "-map".into(), "0:v:0".into()]
        }
        None => {
            let tm = if tm.is_empty() { String::new() } else { format!("{tm},") };
            vec!["-vf".into(), format!("{tm}{scale}format=yuv420p"), "-map".into(), "0:v:0".into()]
        }
    }
}

fn audio_args(audio: Option<usize>, bitrate: &str) -> Vec<String> {
    match audio {
        Some(n) => ["-map", &format!("0:a:{n}?"), "-c:a", "aac", "-b:a", bitrate, "-ac", "2", "-ar", "48000"].map(String::from).to_vec(),
        None => vec!["-an".into()],
    }
}

/// Aperçu : flux continu à partir de `t`
fn preview_args(src: &Source, t: f64, audio: Option<usize>, sub: Option<usize>, height: i64) -> Vec<String> {
    let name = Path::new(&src.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut a: Vec<String> = ["-v", "error", "-hide_banner", "-nostdin"].map(String::from).to_vec();
    a.extend(["-ss".into(), format!("{t:.3}"), "-i".into(), name]);
    a.extend(video_args(src, sub, t, height));
    a.extend(audio_args(audio, "160k"));
    a.extend(
        ["-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-crf", "26", "-g", "48", "-f", "mp4", "-movflags", "frag_keyframe+empty_moov+default_base_moof", "-frag_duration", "1000000", "pipe:1"]
            .map(String::from),
    );
    a
}

/// Export d'un passage, qualité « partage » (H.264 CRF 20, AAC stéréo)
pub fn export_args(src: &Source, seg: &Seg, height: i64, out: &Path) -> Vec<String> {
    let name = Path::new(&src.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut a: Vec<String> = ["-v", "error", "-y", "-hide_banner", "-nostdin", "-nostats", "-progress", "pipe:1"].map(String::from).to_vec();
    a.extend(["-ss".into(), format!("{:.3}", seg.start), "-i".into(), name, "-t".into(), format!("{:.3}", (seg.end - seg.start).max(0.1))]);
    a.extend(video_args(src, seg.sub, seg.start, height));
    a.extend(audio_args(seg.audio, "192k"));
    a.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"].map(String::from));
    a.extend(["-movflags".into(), "+faststart".into(), out.display().to_string()]);
    a
}

/// ffmpeg lancé dans le dossier de la vidéo (le filtre « subtitles » y lit le fichier par son seul nom)
pub fn ffmpeg_in(src: &Source) -> Command {
    let mut c = ff_cmd("ffmpeg.exe");
    if let Some(dir) = Path::new(&src.path).parent() {
        c.current_dir(dir);
    }
    c
}

pub fn source(id: &str) -> Option<Source> {
    SOURCES.lock().unwrap().get(id).cloned()
}

// ---------- serveur d'aperçu ----------

fn server() -> u16 {
    *PORT.get_or_init(|| {
        let Ok(l) = TcpListener::bind("127.0.0.1:0") else { return 0 };
        let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::spawn(move || handle(s));
            }
        });
        port
    })
}

fn query(q: &str, k: &str) -> Option<String> {
    q.split('&').find_map(|kv| kv.split_once('=').filter(|(a, _)| *a == k).map(|(_, b)| b.to_string()))
}

fn handle(mut s: TcpStream) {
    let mut reader = std::io::BufReader::new(match s.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    // Reste de l'en-tête
    loop {
        let mut l = String::new();
        if reader.read_line(&mut l).map(|n| n == 0).unwrap_or(true) || l == "\r\n" {
            break;
        }
    }
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Allow-Headers: *\r\n";
    if method == "OPTIONS" {
        let _ = s.write_all(format!("HTTP/1.1 204 No Content\r\n{CORS}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
        return;
    }
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    let Some(src) = path.strip_prefix("/v/").and_then(source) else {
        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let num = |k: &str| query(q, k).and_then(|v| v.parse::<f64>().ok());
    let t = num("t").unwrap_or(0.0).clamp(0.0, (src.duration - 0.5).max(0.0));
    let track = |k: &str| num(k).filter(|v| *v >= 0.0).map(|v| v as usize);
    let h = num("h").map(|v| v as i64).unwrap_or(540);
    let args = preview_args(&src, t, track("a"), track("s"), h);
    let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nCache-Control: no-store\r\n{CORS}Connection: close\r\n\r\n").as_bytes());
    if method == "HEAD" {
        return;
    }
    let Ok(mut child) = ffmpeg_in(&src).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { return };
    let mut out = child.stdout.take().unwrap();
    let mut buf = vec![0u8; 256 * 1024];
    // Le lecteur coupe la connexion (pause longue, autre instant, fenêtre fermée) → on arrête ffmpeg
    loop {
        match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if s.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Durée « 1m05 » pour les noms de fichiers
pub fn stamp(t: f64) -> String {
    crate::ytdl::hms(t.max(0.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(esc_filter("Test [Sub's], ep 1.mkv"), r"Test \[Sub\\\'s\]\, ep 1.mkv");
    }

    /// cargo test clip_live -- --ignored --nocapture  (CLIP=chemin d'une vidéo)
    #[test]
    #[ignore]
    fn clip_live() {
        let path = std::env::var("CLIP").unwrap();
        let ff = PathBuf::from(std::env::var("FFDIR").unwrap());
        let src = open(&ff, &path).unwrap();
        println!("{} : {:.0} s, {}x{} {} hdr={} audio={:?} subs={:?}", src.name, src.duration, src.width, src.height, src.vcodec, src.hdr, src.audio.iter().map(|a| &a.lang).collect::<Vec<_>>(), src.subs.iter().map(|s| (&s.title, s.image)).collect::<Vec<_>>());
        // Aperçu par le serveur : quelques secondes de flux
        let t0 = std::time::Instant::now();
        let mut c = TcpStream::connect(("127.0.0.1", server())).unwrap();
        let sub = src.subs.last().map(|s| s.n).map(|n| n as i64).unwrap_or(-1);
        write!(c, "GET /v/{}?t=10&a=0&s={sub}&h=540 HTTP/1.1\r\nHost: x\r\n\r\n", src.id).unwrap();
        let mut got = vec![0u8; 0];
        let mut buf = [0u8; 65536];
        while got.len() < 1_500_000 {
            let n = c.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        println!("aperçu : {} octets en {:?}, début : {:?}", got.len(), t0.elapsed(), String::from_utf8_lossy(&got[..60.min(got.len())]));
        drop(c);
        // Export de deux passages (VF puis VO sous-titrée) et assemblage
        let dir = std::env::temp_dir().join("ofa-clip-out");
        let _ = std::fs::create_dir_all(&dir);
        let segs = [
            Seg { start: 5.0, end: 9.0, audio: Some(0), sub: None },
            Seg { start: 5.0, end: 9.0, audio: Some(src.audio.len().saturating_sub(1)), sub: src.subs.last().map(|s| s.n) },
        ];
        for (i, seg) in segs.iter().enumerate() {
            let out = dir.join(format!("seg{i}.mp4"));
            let st = ffmpeg_in(&src).args(export_args(&src, seg, 720, &out)).stdout(Stdio::null()).status().unwrap();
            println!("passage {i} : {st:?} {} octets", std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0));
            assert!(st.success());
        }
    }
}
