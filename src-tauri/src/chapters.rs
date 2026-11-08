//! Chapitres d'un fichier MKV (« Opening », « Ending », « Preview »…), lus directement dans le fichier.
//!
//! Sert à savoir quand commence le générique de fin, et s'il reste quelque chose après
//! (scène post-générique, aperçu du prochain épisode).
//!
//! Format Matroska (EBML) : Segment → (SeekHead → position des Chapters) ou Chapters directement
//! → EditionEntry → ChapterAtom (ChapterTimeStart, ChapterTimeEnd, ChapterDisplay → ChapString).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    pub start: f64,
    pub end: Option<f64>,
    pub name: String,
}

const SEGMENT: u64 = 0x18538067;
const SEEKHEAD: u64 = 0x114D9B74;
const SEEK: u64 = 0x4DBB;
const SEEK_ID: u64 = 0x53AB;
const SEEK_POS: u64 = 0x53AC;
const CHAPTERS: u64 = 0x1043A770;
const CLUSTER: u64 = 0x1F43B675;
const EDITION: u64 = 0x45B9;
const ATOM: u64 = 0xB6;
const TIME_START: u64 = 0x91;
const TIME_END: u64 = 0x92;
const DISPLAY: u64 = 0x80;
const STRING: u64 = 0x85;
const HIDDEN: u64 = 0x98;

/// Entier EBML de longueur variable. `keep_marker` : vrai pour les identifiants.
fn vint(r: &mut impl Read, keep_marker: bool) -> Option<(u64, usize)> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b).ok()?;
    let len = b[0].leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    let mut v = if keep_marker { b[0] as u64 } else { (b[0] as u64) & ((1u64 << (8 - len)) - 1) };
    for _ in 1..len {
        r.read_exact(&mut b).ok()?;
        v = (v << 8) | b[0] as u64;
    }
    Some((v, len))
}

/// En-tête d'élément : (id, taille des données ; None = taille inconnue, longueur de l'en-tête)
fn header(r: &mut impl Read) -> Option<(u64, Option<u64>, usize)> {
    let (id, a) = vint(r, true)?;
    let (size, b) = vint(r, false)?;
    let unknown = size == (1u64 << (7 * b)) - 1;
    Some((id, (!unknown).then_some(size), a + b))
}

fn uint(data: &[u8]) -> u64 {
    data.iter().fold(0, |v, b| (v << 8) | *b as u64)
}

/// Enfants directs d'un élément déjà lu en mémoire
fn children(mut data: &[u8]) -> Vec<(u64, &[u8])> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let mut cur = std::io::Cursor::new(data);
        let Some((id, Some(size), hl)) = header(&mut cur) else { break };
        let end = hl + size as usize;
        if end > data.len() {
            break;
        }
        out.push((id, &data[hl..end]));
        data = &data[end..];
    }
    out
}

fn parse_chapters(data: &[u8]) -> Vec<Chapter> {
    let mut out = Vec::new();
    for (id, edition) in children(data) {
        if id != EDITION {
            continue;
        }
        for (id, atom) in children(edition) {
            if id != ATOM {
                continue;
            }
            let mut c = Chapter { start: 0.0, end: None, name: String::new() };
            let mut hidden = false;
            for (id, v) in children(atom) {
                match id {
                    TIME_START => c.start = uint(v) as f64 / 1e9,
                    TIME_END => c.end = Some(uint(v) as f64 / 1e9),
                    HIDDEN => hidden = uint(v) == 1,
                    DISPLAY if c.name.is_empty() => {
                        if let Some((_, s)) = children(v).into_iter().find(|(id, _)| *id == STRING) {
                            c.name = String::from_utf8_lossy(s).trim().to_string();
                        }
                    }
                    _ => {}
                }
            }
            if !hidden {
                out.push(c);
            }
        }
        // Première édition seulement (les autres sont des variantes)
        if !out.is_empty() {
            break;
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

fn read_at(f: &mut File, pos: u64, max: u64) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(pos)).ok()?;
    let (id, size, hl) = header(f)?;
    if id != CHAPTERS {
        return None;
    }
    let size = size?.min(max);
    let mut buf = vec![0u8; size as usize];
    f.read_exact(&mut buf).ok()?;
    let _ = hl;
    Some(buf)
}

/// Chapitres d'un .mkv (liste vide si pas de chapitres ou autre format).
pub fn read(path: &Path) -> Vec<Chapter> {
    let Ok(mut f) = File::open(path) else { return Vec::new() };
    // En-tête EBML
    let Some((0x1A45DFA3, Some(size), _)) = header(&mut f) else { return Vec::new() };
    if f.seek(SeekFrom::Current(size as i64)).is_err() {
        return Vec::new();
    }
    let Some((SEGMENT, _, _)) = header(&mut f) else { return Vec::new() };
    let Ok(seg_start) = f.stream_position() else { return Vec::new() };
    // Éléments du Segment jusqu'au premier Cluster (les vidéos commencent là)
    let mut chapters_pos = None;
    for _ in 0..64 {
        let Ok(at) = f.stream_position() else { break };
        let Some((id, Some(size), _)) = header(&mut f) else { break };
        let Ok(data_start) = f.stream_position() else { break };
        match id {
            CHAPTERS => {
                let mut buf = vec![0u8; size.min(4 << 20) as usize];
                return if f.read_exact(&mut buf).is_ok() { parse_chapters(&buf) } else { Vec::new() };
            }
            SEEKHEAD if size < 1 << 20 => {
                let mut buf = vec![0u8; size as usize];
                if f.read_exact(&mut buf).is_err() {
                    break;
                }
                for (id, seek) in children(&buf) {
                    if id != SEEK {
                        continue;
                    }
                    let kids = children(seek);
                    let sid = kids.iter().find(|(i, _)| *i == SEEK_ID).map(|(_, v)| uint(v));
                    let spos = kids.iter().find(|(i, _)| *i == SEEK_POS).map(|(_, v)| uint(v));
                    if let (Some(CHAPTERS), Some(p)) = (sid, spos) {
                        chapters_pos = Some(seg_start + p);
                    }
                }
            }
            CLUSTER => break,
            _ => {}
        }
        let _ = at;
        if f.seek(SeekFrom::Start(data_start + size)).is_err() {
            break;
        }
    }
    // Chapitres rangés ailleurs (souvent en fin de fichier) : on y saute grâce au SeekHead
    chapters_pos.and_then(|p| read_at(&mut f, p, 4 << 20)).map(|b| parse_chapters(&b)).unwrap_or_default()
}

/// Générique de fin d'après les chapitres, et ce qui le suit.
#[derive(Debug, Clone, PartialEq)]
pub struct Ending {
    pub start: f64,
    pub end: f64,
    /// Ce qui reste après le générique (en secondes) et si c'est un simple aperçu du prochain épisode
    pub after: f64,
    pub after_is_preview: bool,
}

static RE_ENDING: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^\s*(ed\b|ed\s*\d|ending|end\s*credits|credits?|outro|générique|generique)").unwrap());
static RE_PREVIEW: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)(preview|next\s*ep|aperçu|apercu|prochain|yokoku|予告)").unwrap());

/// `length` : durée du fichier (s)
pub fn ending(chapters: &[Chapter], length: f64) -> Option<Ending> {
    let i = chapters.iter().rposition(|c| RE_ENDING.is_match(&c.name))?;
    let c = &chapters[i];
    let end = c.end.or_else(|| chapters.get(i + 1).map(|n| n.start)).unwrap_or(length).min(length);
    // Le générique doit être dans la seconde moitié (sinon c'est un chapitre mal nommé)
    if length > 0.0 && c.start < length * 0.5 {
        return None;
    }
    let rest: Vec<&Chapter> = chapters[i + 1..].iter().filter(|n| n.start < length - 1.0).collect();
    let after = (length - end).max(0.0);
    let after_is_preview = !rest.is_empty() && rest.iter().all(|n| RE_PREVIEW.is_match(&n.name) || n.end.unwrap_or(length) - n.start < 10.0);
    Some(Ending { start: c.start, end, after, after_is_preview })
}

static RE_OPENING: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^\s*(op\b|op\s*\d|opening|générique de début|generique de debut)").unwrap());
static RE_INTRO: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^\s*intro").unwrap());

/// Opening (début, fin en secondes) d'après les chapitres. Un chapitre « Intro » ne compte que s'il n'y a
/// pas de chapitre « Opening / OP » et qu'il a la durée d'un opening (60–130 s) : chez certains, « Intro »
/// est le début de l'épisode avant l'opening.
pub fn opening(chapters: &[Chapter], length: f64) -> Option<(f64, f64)> {
    let span = |i: usize| {
        let c = &chapters[i];
        let end = c.end.or_else(|| chapters.get(i + 1).map(|n| n.start)).unwrap_or(length);
        (c.start, end)
    };
    let i = chapters.iter().position(|c| RE_OPENING.is_match(&c.name)).or_else(|| {
        chapters.iter().enumerate().position(|(i, c)| {
            let (s, e) = span(i);
            RE_INTRO.is_match(&c.name) && (60.0..=130.0).contains(&(e - s))
        })
    })?;
    let (s, e) = span(i);
    // Dans la première moitié, et d'une durée plausible
    (e > s + 20.0 && (length <= 0.0 || s < length * 0.5)).then_some((s, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(start: f64, end: f64, name: &str) -> Chapter {
        Chapter { start, end: Some(end), name: name.into() }
    }

    #[test]
    fn endings() {
        // Dreamland : Ending puis 2 s de « Chapter 2 » → rien d'important après
        let d = vec![ch(0.0, 89.0, "Opening"), ch(89.0, 1290.0, "Chapter 1"), ch(1290.0, 1378.0, "Ending"), ch(1378.0, 1380.0, "Chapter 2")];
        let e = ending(&d, 1380.0).unwrap();
        assert_eq!((e.start, e.end), (1290.0, 1378.0));
        assert!(e.after < 15.0);
        // Bocchi : ED puis Preview
        let b = vec![ch(0.0, 221.0, "Intro"), ch(221.0, 311.0, "OP"), ch(311.0, 619.0, "Part A"), ch(619.0, 1324.9, "Part B"), ch(1324.9, 1415.0, "ED"), ch(1415.0, 1421.1, "Preview")];
        let e = ending(&b, 1421.1).unwrap();
        assert_eq!(e.start, 1324.9);
        assert!(e.after_is_preview);
        // Scène après le générique
        let s = vec![ch(0.0, 1200.0, "Part A"), ch(1200.0, 1290.0, "Credits"), ch(1290.0, 1380.0, "Epilogue")];
        let e = ending(&s, 1380.0).unwrap();
        assert!(e.after >= 85.0 && !e.after_is_preview);
        // « Opening » n'est pas un ending ; un « ED » en début de fichier non plus
        assert!(ending(&[ch(0.0, 90.0, "Opening")], 1400.0).is_none());
    }

    #[test]
    fn openings() {
        // Dreamland : « Opening » dès le début
        let d = vec![ch(0.0, 89.0, "Opening"), ch(89.0, 1290.0, "Chapter 1"), ch(1290.0, 1378.0, "Ending")];
        assert_eq!(opening(&d, 1380.0), Some((0.0, 89.0)));
        // Bocchi : « Intro » (3 min 41 d'épisode) puis « OP » → c'est OP l'opening
        let b = vec![ch(0.0, 221.0, "Intro"), ch(221.0, 311.0, "OP"), ch(311.0, 619.0, "Part A")];
        assert_eq!(opening(&b, 1421.0), Some((221.0, 311.0)));
        // Jaadugar : pas d'« OP », « Intro » de 90 s = l'opening
        let j = vec![ch(0.0, 14.0, "Chapter 1"), ch(14.0, 104.0, "Intro"), ch(104.0, 1327.0, "Chapter 2")];
        assert_eq!(opening(&j, 1423.0), Some((14.0, 104.0)));
        // Ao no Hako : « Prologue » puis « Opening »
        let a = vec![ch(0.0, 92.0, "Prologue"), ch(92.0, 182.0, "Opening"), ch(182.0, 755.0, "Part A")];
        assert_eq!(opening(&a, 1421.0), Some((92.0, 182.0)));
    }

    /// Fichiers réels : cargo test chapters_real -- --ignored --nocapture (CHAPTERS_FILES=a.mkv;b.mkv)
    #[test]
    #[ignore]
    fn chapters_real() {
        for p in std::env::var("CHAPTERS_FILES").unwrap().split(';') {
            let c = read(Path::new(p));
            println!("== {p}");
            for x in &c {
                println!("  {:7.1} → {:?}  {}", x.start, x.end.map(|e| (e * 10.0).round() / 10.0), x.name);
            }
            assert!(!c.is_empty(), "aucun chapitre lu dans {p}");
        }
    }
}
