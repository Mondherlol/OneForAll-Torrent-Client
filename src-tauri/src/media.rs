//! Transforme un torrent brut en « média » : type, titre propre, saison/épisode,
//! qualité, tags, et une clé pour regrouper les éléments similaires.

use crate::qbit::RawTorrent;
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Film,
    Series,
    Anime,
    Game,
    Software,
    Book,
    Audio,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Film => "film",
            Kind::Series => "series",
            Kind::Anime => "anime",
            Kind::Game => "game",
            Kind::Software => "software",
            Kind::Book => "book",
            Kind::Audio => "audio",
            Kind::Other => "other",
        }
    }
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Downloading,
    Stalled,
    Seeding,
    Done,
    Paused,
    Queued,
    Checking,
    Error,
}

#[derive(Serialize, Clone, Debug)]
pub struct Media {
    pub hash: String,
    pub name: String,
    pub title: String,
    pub kind: Kind,
    pub group_key: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub year: Option<u32>,
    pub quality: Option<String>,
    pub tags: Vec<String>,
    pub status: Status,
    pub state: String,
    pub progress: f64,
    pub size: i64,
    pub total_size: i64,
    pub completed: i64,
    pub seq_dl: bool,
    pub magnet: String,
    pub dlspeed: i64,
    pub upspeed: i64,
    pub eta: i64,
    pub added_on: i64,
    pub completion_on: i64,
    pub save_path: String,
    pub content_path: String,
    pub category: String,
    pub ratio: f64,
    pub seeds: i64,
    pub peers: i64,
    /// Torrent partiel : fichiers sélectionnés / total (vidéos si possible)
    pub files_sel: Option<u32>,
    pub files_total: Option<u32>,
    /// Jeu/logiciel déjà installé sur le PC
    pub installed: Option<crate::installed::Program>,
    /// Format de jeu console (« NSP », « XCI »…) et console (« Switch »…)
    pub format: Option<String>,
    pub platform: Option<String>,
    /// Sources dans tout l'essaim (pas seulement connectées)
    pub seeds_total: i64,
    pub peers_total: i64,
    pub availability: f64,
}

fn re(s: &str) -> Regex {
    Regex::new(s).expect("regex valide")
}

static RE_EXT: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\.(mkv|mp4|avi|m4v|cbz|cbr|epub|mobi|azw3|pdf|nsp|nsz|xci|iso|zip|rar|7z|mp3|flac|m4a|m4b)$"));
static RE_LEADING_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*\[[^\]]*\]\s*"));
static RE_SE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bS(\d{1,2})[ ._-]?E(\d{1,4})\b"));
static RE_S: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:S|Saison |Season )(\d{1,2})\b"));
static RE_YEAR: LazyLock<Regex> = LazyLock::new(|| re(r"\b((?:19|20)\d{2})\b"));
static RE_RES: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(2160p|1080p|720p|576p|480p|4k|uhd)\b"));
static RE_CUT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?ix)
        \bS\d{1,2}(?:[ ._-]?E\d{1,4})?\b
      | \b(?:19|20)\d{2}\b
      | \b(?:2160p|1080p|720p|576p|480p|4k|uhd|4klight|hdlight|multi|vff|vf2|vfq|vfi|vf|vostfr|vost|french|truefrench|subfrench
          |int[eé]grale|complete|web|web-?dl|webrip|bluray|bdrip|hdtv|x26[45]|h\.?26[45]|hevc|hybrid
          |repack|fitgirl|dodi|elamigos|multilingual|portable|saison|season|fr|mp3|flac)\b
      | \(v?\d | \[ | \{
    ")
});

static RE_BOOK: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\.(cbz|cbr|epub|mobi|azw3|pdf)$|\b(manga|comics?|ebook|scans?)\b"));
static RE_AUDIO: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\.(mp3|flac|m4a|m4b|ogg|wav)$|\b(mp3|flac|\d{3}\s?kbps|album|discography|discographie|audiobook|livre audio|ost)\b")
});
static RE_GAME: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(fitgirl|dodi|elamigos|gog|codex|skidrow|tenoke|empress|razor1911|steamrip|kaoskrew|plaza)\b|\.(nsp|nsz|xci)$|\[0100[0-9a-f]{12}\]")
});
static RE_CONSOLE_EXT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\.(nsp|nsz|xci|xcz|3ds|cia|nds|gba|wbfs|rvz|cso)$"));
static RE_SWITCH: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\.(nsp|nsz|xci)$|\[0100[0-9a-f]{12}\]|\bswitch\b"));
static RE_SOFT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(adobe|autodesk|microsoft|office|photoshop|illustrator|premiere|lightroom|acrobat|multilingual|x64|x86|portable|pre-?activated|keygen|activator)\b|\bv\d+\.\d+")
});
/// Marqueurs typiques des sorties d'animés (plateformes, équipes de fansub, [Équipe] en tête)
static RE_ANIME: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(adn|crunchyroll|wakanim|tsundere(-raws)?|erai-raws|subsplease|horriblesubs|judas|anime|animes)\b|^\s*\[[^\]]+\]")
});
/// Titre qui se termine par un numéro d'épisode (2 à 4 chiffres) : « One Piece 1180 », « Frieren - 12 »
static RE_ABS_EP: LazyLock<Regex> = LazyLock::new(|| re(r"^(.*?\S)\s*(?:-\s*|#|\bE|\bEp\.?\s?|\bEpisode\s)?(\d{2,4})(?:v\d)?$"));
/// « Episode 12 », « Ep 12 », « E12 » n'importe où dans le nom
static RE_EP_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:episode|ep\.?)\s?(\d{1,4})\b"));
static RE_SERIES_WORDS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b(int[eé]grale|saison|season|complete series|batch)\b"));

/// (motif, libellé affiché)
static TAGS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"(?i)\bmulti\b", "MULTI"),
        (r"(?i)\btruefrench\b", "TRUEFRENCH"),
        (r"(?i)\bvff\b", "VFF"),
        (r"(?i)\bvf2\b", "VF2"),
        (r"(?i)\bvfq\b", "VFQ"),
        (r"(?i)\bvostfr\b", "VOSTFR"),
        (r"(?i)\bhdr(10\+?)?\b", "HDR"),
        (r"(?i)\b(dv|dolby ?vision)\b", "DV"),
        (r"(?i)\batmos\b", "Atmos"),
        (r"(?i)\b(x265|h\.?265|hevc)\b", "HEVC"),
        (r"(?i)\bblu-?ray\b", "BluRay"),
        (r"(?i)\bweb(-?dl|rip)?\b", "WEB"),
        (r"(?i)\bfitgirl\b", "FitGirl"),
        (r"(?i)\bdodi\b", "DODI"),
        (r"(?i)\belamigos\b", "ElAmigos"),
    ]
    .into_iter()
    .map(|(p, l)| (re(p), l))
    .collect()
});

/// Remplace les points/underscores par des espaces quand le nom est au format « scène ».
fn humanize(name: &str) -> String {
    let dots = name.matches(['.', '_']).count();
    let spaces = name.matches(' ').count();
    if dots >= 2 && dots > spaces {
        name.replace(['.', '_'], " ")
    } else {
        name.to_string()
    }
}

pub fn clean_title(name: &str) -> String {
    let no_ext = RE_EXT.replace(name, "");
    let human = humanize(&no_ext);
    let body = RE_LEADING_TAG.replace(&human, "").to_string();

    // On coupe au premier marqueur technique qui n'est pas en tout début de nom.
    let cut = RE_CUT
        .find_iter(&body)
        .find(|m| m.start() > 0)
        .map(|m| m.start())
        .unwrap_or(body.len());
    let title = body[..cut].trim_end_matches(|c: char| c.is_whitespace() || "-_.([{,".contains(c)).trim();
    if title.is_empty() {
        body.trim().to_string()
    } else {
        title.to_string()
    }
}

pub fn detect_kind(name: &str, category: &str) -> Kind {
    let cat = category.to_lowercase();
    let human = humanize(name);
    if RE_BOOK.is_match(name) || cat.contains("livre") || cat.contains("book") || cat.contains("manga") {
        return Kind::Book;
    }
    if RE_AUDIO.is_match(&human) || cat.contains("musique") || cat.contains("music") || cat.contains("audio") {
        return Kind::Audio;
    }
    if RE_GAME.is_match(&human) {
        return Kind::Game;
    }
    if RE_SOFT.is_match(&human) || cat.contains("logiciel") || cat.contains("soft") || cat.contains("app") {
        return Kind::Software;
    }
    if RE_SE.is_match(&human)
        || RE_S.is_match(&human)
        || RE_SERIES_WORDS.is_match(&human)
        || cat.contains("serie")
        || cat.contains("série")
        || cat.contains("anime")
        || cat.contains("tv")
    {
        let anime = RE_ANIME.is_match(&human) || cat.contains("anime") || cat.contains("animé");
        return if anime { Kind::Anime } else { Kind::Series };
    }
    if cat.contains("anime") || cat.contains("animé") {
        return Kind::Anime;
    }
    if cat.contains("jeu") || cat.contains("game") {
        return Kind::Game;
    }
    if cat.contains("film") || cat.contains("movie") || RE_RES.is_match(&human) {
        return Kind::Film;
    }
    Kind::Other
}

fn status_of(state: &str) -> Status {
    match state {
        "downloading" | "forcedDL" | "metaDL" | "forcedMetaDL" => Status::Downloading,
        "stalledDL" => Status::Stalled,
        "uploading" | "stalledUP" | "forcedUP" => Status::Seeding,
        "pausedUP" | "stoppedUP" => Status::Done,
        "pausedDL" | "stoppedDL" => Status::Paused,
        "queuedDL" | "queuedUP" => Status::Queued,
        "checkingDL" | "checkingUP" | "checkingResumeData" | "moving" | "allocating" => Status::Checking,
        "error" | "missingFiles" => Status::Error,
        _ => Status::Queued,
    }
}

pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn key_prefix(kind: Kind) -> &'static str {
    match kind {
        Kind::Series | Kind::Anime => "show",
        k => k.as_str(),
    }
}

impl Media {
    /// Change le type (et la clé de groupe qui en dépend).
    pub fn set_kind(&mut self, kind: Kind) {
        if key_prefix(kind) != key_prefix(self.kind) {
            let rest = self.group_key.split_once(':').map(|(_, r)| r.to_string()).unwrap_or_default();
            self.group_key = format!("{}:{rest}", key_prefix(kind));
        }
        self.kind = kind;
    }

    pub fn from_raw(t: RawTorrent) -> Self {
        let human = humanize(&RE_EXT.replace(&t.name, ""));
        let mut kind = detect_kind(&t.name, &t.category);
        let mut title = clean_title(&t.name);
        let console_format = RE_CONSOLE_EXT.captures(&t.name).map(|c| c[1].to_uppercase());
        let console_platform = RE_SWITCH.is_match(&t.name).then(|| "Switch".to_string());

        let (season, mut episode): (Option<u32>, Option<u32>) = match RE_SE.captures(&human) {
            Some(c) => (c[1].parse().ok(), c[2].parse().ok()),
            None => (RE_S.captures(&human).and_then(|c| c[1].parse().ok()), None),
        };

        // Numérotation absolue des animés : « One Piece - 1180 », « One Piece 1180 », « Ep 12 ».
        // Le numéro sort du titre (sinon chaque épisode serait un titre différent, sans affiche).
        if episode.is_none() && season.is_none() {
            if let Some(c) = RE_ABS_EP.captures(&title) {
                let n: u32 = c[2].parse().unwrap_or(0);
                // Un film « Ocean's 11 (2001) » garde son nombre : on ne l'applique que si c'est déjà une série,
                // si le numéro est précédé d'un tiret (« - 1180 »), ou s'il n'y a pas d'année dans le nom
                let dash = c[0][c[1].len()..].contains('-');
                let plausible = matches!(kind, Kind::Series | Kind::Anime) || dash || !RE_YEAR.is_match(&human);
                if plausible && !(1900..=2099).contains(&n) && !matches!(kind, Kind::Game | Kind::Software | Kind::Book | Kind::Audio) {
                    episode = Some(n);
                    title = c[1].trim_end_matches(|ch: char| ch == '-' || ch.is_whitespace()).to_string();
                }
            } else if let Some(c) = RE_EP_WORD.captures(&human) {
                episode = c[1].parse().ok();
            }
            if episode.is_some() && matches!(kind, Kind::Film | Kind::Other) {
                // Un épisode n'est pas un film : série, ou animé si le nom en a les marques
                kind = if RE_ANIME.is_match(&human) { Kind::Anime } else { Kind::Series };
            }
        }
        let year = RE_YEAR.captures(&human).and_then(|c| c[1].parse().ok());
        let quality = RE_RES.captures(&human).map(|c| match c[1].to_lowercase().as_str() {
            "2160p" | "4k" | "uhd" => "4K".to_string(),
            r => r.to_string(),
        });
        let mut tags: Vec<String> = TAGS
            .iter()
            .filter(|(r, _)| r.is_match(&human))
            .map(|(_, l)| l.to_string())
            .collect();
        if RE_SWITCH.is_match(&t.name) {
            tags.insert(0, "Switch".into());
        }

        // Séries et animés partagent la même clé : un même titre reste groupé
        let kind_key = key_prefix(kind);
        Self {
            group_key: format!("{kind_key}:{}", normalize(&title)),
            hash: t.hash,
            title,
            kind,
            season,
            episode,
            year,
            quality,
            tags,
            status: status_of(&t.state),
            state: t.state,
            progress: t.progress,
            size: t.size,
            total_size: t.total_size,
            completed: t.completed,
            seq_dl: t.seq_dl,
            magnet: t.magnet_uri,
            dlspeed: t.dlspeed,
            upspeed: t.upspeed,
            eta: t.eta,
            added_on: t.added_on,
            completion_on: t.completion_on,
            save_path: t.save_path,
            content_path: t.content_path,
            category: t.category,
            ratio: t.ratio,
            seeds: t.num_seeds,
            peers: t.num_leechs,
            name: t.name,
            files_sel: None,
            files_total: None,
            installed: None,
            format: console_format,
            platform: console_platform,
            seeds_total: t.num_complete,
            peers_total: t.num_incomplete,
            availability: t.availability,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, cat: &str) -> Media {
        Media::from_raw(RawTorrent { name: name.into(), category: cat.into(), ..Default::default() })
    }

    #[test]
    fn series_are_grouped() {
        let a = m("[Trix] JoJo's Bizarre Adventure (2012) S06E08 [WEB-DL 1080p]", "Series");
        let b = m("JoJos.Bizarre.Adventure.2012.S06E06.La.Destinee.De.Gyro.Zeppeli.MULTi.1080p", "");
        assert_eq!(a.kind, Kind::Anime); // « [Trix] » en tête = sortie d'animé
        assert_eq!(b.kind, Kind::Series);
        assert_eq!(m("Dreamland S01E01 MULTi 1080p ADN WEB-DL AAC2.0 x264-Tsundere-Raws", "").kind, Kind::Anime);
        assert_eq!(a.title, "JoJo's Bizarre Adventure");
        assert_eq!(a.group_key, b.group_key);
        assert_eq!((b.season, b.episode), (Some(6), Some(6)));
    }

    /// `MEDIADECK_NAMES=fichier.tsv cargo test dump -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump() {
        let path = std::env::var("MEDIADECK_NAMES").expect("MEDIADECK_NAMES");
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let (name, cat) = line.split_once('\t').unwrap_or((line, ""));
            let x = m(name, cat);
            println!("{:<9} | {:<34} | {:<40} | {:?}", format!("{:?}", x.kind), x.title, x.group_key, x.tags);
        }
    }

    #[test]
    fn absolute_episodes() {
        for name in [
            "[Erai-raws] One Piece - 1180 [1080p CR WEB-DL AVC AAC][MultiSub][ABCD1234]",
            "One.Piece.1180.VOSTFR.1080p.WEB.x264-Tsundere-Raws",
            "One Piece 1180 VOSTFR 1080p",
        ] {
            let x = m(name, "");
            assert_eq!(x.title, "One Piece", "{name}");
            assert_eq!(x.episode, Some(1180), "{name}");
            assert!(matches!(x.kind, Kind::Series | Kind::Anime), "{name} → {:?}", x.kind);
        }
        // Tous les épisodes dans le même groupe
        assert_eq!(m("One Piece 1179 VOSTFR 1080p", "").group_key, m("One Piece 1180 VOSTFR 1080p", "").group_key);
        // Les films avec un nombre dans le titre restent des films
        let o = m("Ocean's 11 (2001) MULTi 1080p BluRay", "");
        assert_eq!((o.kind, o.title.as_str()), (Kind::Film, "Ocean's 11"));
        assert_eq!(m("Deadpool 2 (2018) MULTi 2160p", "").kind, Kind::Film);
    }

    #[test]
    fn kinds() {
        assert_eq!(m("Deadpool 2 (2018) Super Duper Cut MULTi VFF 2160p", "").kind, Kind::Film);
        assert_eq!(m("Deadpool 2 (2018) Super Duper Cut MULTi VFF 2160p", "").title, "Deadpool 2");
        assert_eq!(m("Blue Prince (v1.06.1)-ElAmigos", "Jeux").kind, Kind::Game);
        assert_eq!(m("Blue Prince (v1.06.1)-ElAmigos", "Jeux").title, "Blue Prince");
        assert_eq!(m("Adobe Illustrator 2026 (v30.8.2) Multilingual", "Jeux").kind, Kind::Software);
        assert_eq!(m("Look Back.cbz", "Films").kind, Kind::Book);
        assert_eq!(m("Sherlock.Holmes.integrale.2007.FR.MP3.[128kbps]", "").kind, Kind::Audio);
        assert_eq!(m("Black.Clover.iNTEGRALE.MULTi.1080p.BluRay.x264-AMB3R", "").title, "Black Clover");
        let ac = m("Animal Crossing New Horizons [01006F8002326800] [v2097152].nsp", "");
        assert_eq!(ac.kind, Kind::Game);
        assert_eq!(ac.title, "Animal Crossing New Horizons");
        assert!(ac.tags.contains(&"Switch".to_string()));
        assert_eq!(m("GOD EATER 3", "Jeux").group_key, m("God Eater 3 [FitGirl Repack]", "Jeux").group_key);
    }
}
