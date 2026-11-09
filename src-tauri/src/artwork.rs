//! Recherche d'affiches, sans clé API par défaut :
//! séries → TVmaze, animés → AniList, films → iTunes puis Wikipédia (ou TMDB si une clé est fournie),
//! jeux → IGDB (si l'app est compilée avec ses identifiants) puis Steam puis Wikipédia, livres/mangas → AniList, audio → iTunes.
//! Les résultats sont mis en cache dans %APPDATA%\OneForAll\artwork.json.
//! La source permet aussi de corriger le type : un « autre » trouvé sur Steam est un jeu,
//! une série que TVmaze classe en animation japonaise est un animé.

use crate::config::data_dir;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Art {
    pub url: String,
    /// Image de secours si la première ne charge pas
    pub fallback: Option<String>,
    /// Type corrigé grâce à la source (« anime », « game »)
    #[serde(default)]
    pub kind: Option<String>,
}

impl Art {
    fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), fallback: None, kind: None }
    }
    fn fallback(mut self, f: impl Into<String>) -> Self {
        self.fallback = Some(f.into());
        self
    }
    fn kind(mut self, k: &str) -> Self {
        self.kind = Some(k.into());
        self
    }
}

pub struct Artwork {
    client: Client,
    cache: Mutex<HashMap<String, Option<Art>>>,
    /// Jeton IGDB (Twitch) et sa date d'expiration
    igdb_token: Mutex<Option<(String, std::time::Instant)>>,
}

// Identifiants IGDB intégrés à la compilation (.env.local, jamais dans le dépôt).
// Avec le secret, l'app obtient et renouvelle elle-même son jeton ; sinon le jeton fixe est utilisé.
const IGDB_ID: Option<&str> = option_env!("ONEFORALL_IGDB_CLIENT_ID");
const IGDB_SECRET: Option<&str> = option_env!("ONEFORALL_IGDB_SECRET");
const IGDB_TOKEN: Option<&str> = option_env!("ONEFORALL_IGDB_TOKEN");

fn igdb_enabled() -> bool {
    IGDB_ID.is_some_and(|s| !s.is_empty()) && (IGDB_SECRET.is_some_and(|s| !s.is_empty()) || IGDB_TOKEN.is_some_and(|s| !s.is_empty()))
}

fn cache_path() -> std::path::PathBuf {
    data_dir().join("artwork.json")
}

/// Mots en minuscules, sans ponctuation ni accents (« Pokémon » = « pokemon »).
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| fold(&w.to_lowercase()))
        .collect()
}

/// Lettres sans accents (les plus courantes)
fn fold(w: &str) -> String {
    w.chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' | 'ā' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'ē' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ō' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        })
        .collect()
}

/// Distance d'édition (lettres ajoutées, retirées ou changées)
fn lev(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Deux mots « pareils » malgré une faute de frappe : 1 lettre d'écart dès 5 lettres, 2 dès 8.
fn same_word(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let n = a.chars().count().min(b.chars().count());
    let tol = if n >= 8 { 2 } else if n >= 5 { 1 } else { 0 };
    tol > 0 && lev(a, b) <= tol
}

/// Tous les mots cherchés (sauf les tout petits) sont dans le titre trouvé : nom raccourci
/// (« Zelda Ocarina » → « The Legend of Zelda: Ocarina of Time »).
fn contains_all(query: &str, found: &str) -> bool {
    let (a, b) = (words(query), words(found));
    let a: Vec<&String> = a.iter().filter(|w| w.chars().count() > 2).collect();
    !a.is_empty() && a.iter().all(|w| b.iter().any(|x| same_word(w, x)))
}

/// Proximité entre deux titres (0 à 1) : mots en commun, 1 si identiques.
fn closeness(query: &str, found: &str) -> f64 {
    let (a, b) = (words(query), words(found));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a.concat() == b.concat() {
        return 1.0;
    }
    let common = a.iter().filter(|w| b.iter().any(|x| same_word(w, x))).count() as f64;
    0.99 * common / ((a.len() + b.len()) as f64 - common)
}

/// Nom d'un jeu IGDB et ses noms alternatifs (autres langues, régions)
fn names_of(g: &Value) -> Vec<String> {
    std::iter::once(g["name"].as_str().unwrap_or_default().to_string())
        .chain(g["alternative_names"].as_array().into_iter().flatten().filter_map(|n| n["name"].as_str().map(str::to_string)))
        .collect()
}

fn igdb_img(id: &str, size: &str) -> String {
    format!("https://images.igdb.com/igdb/image/upload/{size}/{id}.jpg")
}

/// Une image proposée dans la fenêtre « Choisir l'image »
#[derive(Serialize, Clone, Debug)]
pub struct Choice {
    pub url: String,
    pub fallback: Option<String>,
    /// « IGDB », « TMDB »…
    pub source: &'static str,
    /// Titre de l'œuvre trouvée
    pub title: String,
    /// Année, consoles…
    pub sub: String,
}

/// Vérifie qu'un résultat correspond vraiment au titre recherché.
pub fn similar(query: &str, found: &str) -> bool {
    let (a, b) = (words(query), words(found));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (ja, jb) = (a.concat(), b.concat());
    if ja == jb || jb.starts_with(&ja) || ja.starts_with(&jb) {
        return true;
    }
    let common = a.iter().filter(|w| b.iter().any(|x| same_word(w, x))).count() as f64;
    let union = (a.len() + b.len()) as f64 - common;
    common / union >= 0.5
}

/// Titre utilisable pour une recherche : sans parenthèses ni crochets.
fn query_of(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// « Sousou no Frieren 2nd Season » → « Sousou no Frieren »
pub fn strip_season(q: &str) -> String {
    let re = regex::Regex::new(r"(?i)\s*(\d+(st|nd|rd|th)(\s+season)?|season\s*\d+|saison\s*\d+|s\d{1,2}|part\s*\d+|cour\s*\d+)\s*$").unwrap();
    re.replace(q, "").trim().to_string()
}

impl Artwork {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("OneForAll/0.3")
            .build()
            .expect("client HTTP");
        let mut cache: HashMap<String, Option<Art>> = std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // Les « rien trouvé » ne sont gardés que le temps d'une session : on réessaie au prochain lancement
        cache.retain(|_, v| v.is_some());
        // Première fois avec IGDB : on oublie les images de jeux venues de Steam / Wikipédia
        let marker = data_dir().join("artwork.igdb");
        if igdb_enabled() && !marker.is_file() {
            cache.retain(|_, v| v.as_ref().is_some_and(|a| !(a.url.contains("steamstatic") || a.url.contains("wikimedia") || a.url.contains("wikipedia"))));
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(&marker, "1");
        }
        Self { client, cache: Mutex::new(cache), igdb_token: Mutex::new(None) }
    }

    pub async fn get(&self, key: &str, kind: &str, title: &str, year: Option<u32>, tmdb_key: &str) -> Option<Art> {
        if let Some(hit) = self.cache.lock().unwrap().get(key) {
            return hit.clone();
        }
        let q = query_of(title);
        // Une erreur réseau n'est pas mise en cache (on réessaiera plus tard).
        let found = self.lookup(kind, &q, year, tmdb_key).await.ok()?;
        let mut cache = self.cache.lock().unwrap();
        cache.insert(key.to_string(), found.clone());
        let found_only: HashMap<&String, &Option<Art>> = cache.iter().filter(|(_, v)| v.is_some()).collect();
        if let Ok(s) = serde_json::to_string(&found_only) {
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(cache_path(), s);
        }
        found
    }

    /// Affiche déjà connue pour cette clé (sans requête réseau).
    pub fn cached(&self, key: &str) -> Option<Art> {
        self.cache.lock().unwrap().get(key).cloned().flatten()
    }

    /// Télécharge l'affiche dans %APPDATA%\OneForAll\posters (pour les notifications Windows,
    /// qui n'acceptent que des images locales). Renvoie le chemin du fichier.
    pub async fn poster_file(&self, key: &str) -> Option<std::path::PathBuf> {
        let art = self.cached(key)?;
        let dir = data_dir().join("posters");
        std::fs::create_dir_all(&dir).ok()?;
        let name: String = key.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let path = dir.join(format!("{name}.jpg"));
        if path.is_file() {
            return Some(path);
        }
        if let Some(b64) = art.url.strip_prefix("data:image/jpeg;base64,") {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
            return std::fs::write(&path, bytes).ok().map(|_| path);
        }
        for url in [Some(art.url), art.fallback].into_iter().flatten() {
            let Ok(res) = self.client.get(&url).send().await else { continue };
            if !res.status().is_success() {
                continue;
            }
            let Ok(bytes) = res.bytes().await else { continue };
            if std::fs::write(&path, &bytes).is_ok() {
                return Some(path);
            }
        }
        None
    }

    /// Image choisie par l'utilisateur : remplace l'image automatique (gardée en cache).
    pub fn set(&self, key: &str, art: Art) {
        let mut cache = self.cache.lock().unwrap();
        cache.insert(key.to_string(), Some(art));
        let found_only: HashMap<&String, &Option<Art>> = cache.iter().filter(|(_, v)| v.is_some()).collect();
        if let Ok(s) = serde_json::to_string(&found_only) {
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(cache_path(), s);
        }
        drop(cache);
        // L'affiche des notifications sera refaite avec la nouvelle image
        let name: String = key.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let _ = std::fs::remove_file(data_dir().join("posters").join(format!("{name}.jpg")));
    }

    /// Images possibles pour un titre, toutes sources confondues selon le type (les plus probables d'abord).
    pub async fn choices(&self, kind: &str, q: &str, year: Option<u32>, tmdb: &str) -> Vec<Choice> {
        let q = query_of(q);
        let q = q.as_str();
        let none = || async { Vec::<Choice>::new() };
        let (a, b, c, d) = match kind {
            "game" | "other" | "software" => tokio::join!(self.igdb_choices(q), self.steam_choices(q), self.wiki_choices("en", q, "video game"), self.wiki_choices("fr", q, "jeu vidéo")),
            "film" => tokio::join!(self.tmdb_choices("movie", q, year, tmdb), self.itunes_choices(q, "movie", "600x900bb"), self.wiki_choices("en", q, "film"), self.wiki_choices("fr", q, "film")),
            "anime" => tokio::join!(self.anilist_choices(q, "ANIME"), self.tmdb_choices("tv", q, None, tmdb), self.tvmaze_choices(q), none()),
            "series" => tokio::join!(self.tmdb_choices("tv", q, None, tmdb), self.tvmaze_choices(q), self.anilist_choices(q, "ANIME"), none()),
            "book" => tokio::join!(self.anilist_choices(q, "MANGA"), self.itunes_choices(q, "ebook", "600x900bb"), self.wiki_choices("fr", q, "manga"), none()),
            "audio" => tokio::join!(self.itunes_choices(q, "audiobook", "600x600bb"), self.itunes_choices(q, "music", "600x600bb"), none(), none()),
            _ => tokio::join!(self.tmdb_choices("movie", q, year, tmdb), self.igdb_choices(q), self.tvmaze_choices(q), self.anilist_choices(q, "ANIME")),
        };
        let mut seen = std::collections::HashSet::new();
        [a, b, c, d].into_iter().flatten().filter(|x| seen.insert(x.url.clone())).collect()
    }

    async fn igdb_choices(&self, q: &str) -> Vec<Choice> {
        self.igdb_games(q, 12)
            .await
            .into_iter()
            .filter_map(|(_, g)| {
                let cover = g["cover"]["image_id"].as_str()?;
                let year = g["first_release_date"].as_i64().map(|t| (1970 + t / 31_556_952).to_string());
                let plats: Vec<&str> = g["platforms"].as_array().into_iter().flatten().filter_map(|p| p["abbreviation"].as_str()).take(3).collect();
                let sub = year.into_iter().chain((!plats.is_empty()).then(|| plats.join(", "))).collect::<Vec<_>>().join(" · ");
                Some(Choice { url: igdb_img(cover, "t_cover_big_2x"), fallback: Some(igdb_img(cover, "t_cover_big")), source: "IGDB", title: g["name"].as_str().unwrap_or_default().into(), sub })
            })
            .collect()
    }

    async fn steam_choices(&self, q: &str) -> Vec<Choice> {
        let req = self.client.get("https://store.steampowered.com/api/storesearch/").query(&[("term", q), ("l", "french"), ("cc", "FR")]);
        let Ok(v) = self.json(req).await else { return Vec::new() };
        let items: Vec<(i64, String)> = v["items"].as_array().into_iter().flatten().filter_map(|r| Some((r["id"].as_i64()?, r["name"].as_str()?.to_string()))).take(6).collect();
        let mut out = Vec::new();
        for (id, name) in items {
            let art = match self.steam_assets(id).await {
                Some(a) => a,
                None => {
                    let base = format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}");
                    Art::new(format!("{base}/library_600x900.jpg")).fallback(format!("{base}/header.jpg"))
                }
            };
            out.push(Choice { url: art.url, fallback: art.fallback, source: "Steam", title: name, sub: String::new() });
        }
        out
    }

    async fn wiki_choices(&self, lang: &str, q: &str, hint: &str) -> Vec<Choice> {
        let search = format!("{q} {hint}");
        let req = self.client.get(format!("https://{lang}.wikipedia.org/w/api.php")).query(&[
            ("action", "query"),
            ("format", "json"),
            ("generator", "search"),
            ("gsrsearch", search.as_str()),
            ("gsrlimit", "6"),
            ("prop", "pageimages"),
            ("piprop", "thumbnail"),
            ("pithumbsize", "600"),
            ("pilicense", "any"),
        ]);
        let Ok(v) = self.json(req).await else { return Vec::new() };
        let mut pages: Vec<&Value> = v["query"]["pages"].as_object().into_iter().flat_map(|m| m.values()).collect();
        pages.sort_by_key(|p| p["index"].as_i64().unwrap_or(99));
        let src = if lang == "fr" { "Wikipédia FR" } else { "Wikipédia" };
        pages
            .into_iter()
            .filter_map(|p| Some(Choice { url: p["thumbnail"]["source"].as_str()?.into(), fallback: None, source: src, title: p["title"].as_str()?.into(), sub: String::new() }))
            .collect()
    }

    /// TMDB : affiches des résultats, plus toutes les affiches (langues, éditions) du premier.
    async fn tmdb_choices(&self, kind: &str, q: &str, year: Option<u32>, key: &str) -> Vec<Choice> {
        if key.is_empty() {
            return Vec::new();
        }
        let y = year.map(|y| y.to_string()).unwrap_or_default();
        let mut query = vec![("api_key", key), ("query", q), ("language", "fr-FR")];
        if !y.is_empty() && kind == "movie" {
            query.push(("year", &y));
        }
        let Ok(v) = self.json(self.client.get(format!("https://api.themoviedb.org/3/search/{kind}")).query(&query)).await else { return Vec::new() };
        let img = |p: &str| (format!("https://image.tmdb.org/t/p/w500{p}"), format!("https://image.tmdb.org/t/p/w342{p}"));
        let name = |r: &Value| r["title"].as_str().or(r["name"].as_str()).unwrap_or_default().to_string();
        let date = |r: &Value| r["release_date"].as_str().or(r["first_air_date"].as_str()).unwrap_or_default().chars().take(4).collect::<String>();
        let results: Vec<&Value> = v["results"].as_array().into_iter().flatten().filter(|r| r["poster_path"].is_string()).take(8).collect();
        let mut out: Vec<Choice> = Vec::new();
        for (i, r) in results.iter().enumerate() {
            let (u, f) = img(r["poster_path"].as_str().unwrap_or_default());
            out.push(Choice { url: u, fallback: Some(f), source: "TMDB", title: name(r), sub: date(r) });
            if i > 0 {
                continue;
            }
            // Les autres affiches du premier résultat juste après lui
            let id = r["id"].as_i64().unwrap_or_default();
            let url = format!("https://api.themoviedb.org/3/{kind}/{id}/images");
            if let Ok(imgs) = self.json(self.client.get(url).query(&[("api_key", key), ("include_image_language", "fr,en,null,ja")])).await {
                for p in imgs["posters"].as_array().into_iter().flatten().take(16) {
                    let Some(path) = p["file_path"].as_str() else { continue };
                    let (u, f) = img(path);
                    let lang = p["iso_639_1"].as_str().map(|l| l.to_uppercase()).unwrap_or_else(|| "sans texte".into());
                    out.push(Choice { url: u, fallback: Some(f), source: "TMDB", title: name(r), sub: [date(r), lang].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ") });
                }
            }
        }
        out
    }

    async fn itunes_choices(&self, q: &str, media: &str, size: &str) -> Vec<Choice> {
        let country = if media == "movie" { "US" } else { "FR" };
        let mut query = vec![("term", q), ("limit", "15"), ("country", country)];
        if media != "movie" {
            query.push(("media", media));
        }
        let Ok(v) = self.json(self.client.get("https://itunes.apple.com/search").query(&query)).await else { return Vec::new() };
        v["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| media != "movie" || r["kind"].as_str() == Some("feature-movie"))
            .filter_map(|r| {
                let img = r["artworkUrl100"].as_str()?;
                let title = r["trackName"].as_str().or(r["collectionName"].as_str())?.to_string();
                let sub = r["releaseDate"].as_str().unwrap_or_default().chars().take(4).collect();
                Some(Choice { url: img.replace("100x100bb", size), fallback: Some(img.into()), source: "iTunes", title, sub })
            })
            .take(8)
            .collect()
    }

    async fn tvmaze_choices(&self, q: &str) -> Vec<Choice> {
        let Ok(v) = self.json(self.client.get("https://api.tvmaze.com/search/shows").query(&[("q", q)])).await else { return Vec::new() };
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let show = &r["show"];
                let img = show["image"]["original"].as_str().or(show["image"]["medium"].as_str())?;
                Some(Choice {
                    url: img.into(),
                    fallback: show["image"]["medium"].as_str().map(Into::into),
                    source: "TVmaze",
                    title: show["name"].as_str()?.into(),
                    sub: show["premiered"].as_str().unwrap_or_default().chars().take(4).collect(),
                })
            })
            .collect()
    }

    async fn anilist_choices(&self, q: &str, kind: &str) -> Vec<Choice> {
        let body = json!({
            "query": "query($s:String,$t:MediaType){Page(perPage:10){media(search:$s,type:$t){format seasonYear title{romaji english} coverImage{extraLarge large}}}}",
            "variables": { "s": q, "t": kind }
        });
        let Ok(v) = self.json(self.client.post("https://graphql.anilist.co").json(&body)).await else { return Vec::new() };
        v["data"]["Page"]["media"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let img = m["coverImage"]["extraLarge"].as_str().or(m["coverImage"]["large"].as_str())?;
                let title = m["title"]["english"].as_str().or(m["title"]["romaji"].as_str())?.to_string();
                let sub = [m["seasonYear"].as_i64().map(|y| y.to_string()), m["format"].as_str().map(str::to_string)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                Some(Choice { url: img.into(), fallback: m["coverImage"]["large"].as_str().map(Into::into), source: "AniList", title, sub })
            })
            .collect()
    }

    pub fn forget(&self, key: &str) {
        self.cache.lock().unwrap().remove(key);
    }

    /// Clé de groupe → type corrigé par la source d'images
    pub fn kind_hints(&self) -> HashMap<String, String> {
        self.cache
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_ref()?.kind.clone()?)))
            .collect()
    }

    /// Cherche avec le titre complet, puis sans la mention de saison (« 2nd Season », « Part 2 »…).
    async fn lookup(&self, kind: &str, q: &str, year: Option<u32>, tmdb: &str) -> Result<Option<Art>, reqwest::Error> {
        if let Some(a) = self.lookup_once(kind, q, year, tmdb).await? {
            return Ok(Some(a));
        }
        let short = strip_season(q);
        if !short.is_empty() && short != q {
            return self.lookup_once(kind, &short, year, tmdb).await;
        }
        Ok(None)
    }

    async fn lookup_once(&self, kind: &str, q: &str, year: Option<u32>, tmdb: &str) -> Result<Option<Art>, reqwest::Error> {
        // TMDB (si une clé est fournie) d'abord, puis les sources sans clé s'il ne trouve rien
        if !tmdb.is_empty() {
            let hit = match kind {
                "film" => self.tmdb("movie", q, year, tmdb).await?,
                "series" | "anime" => self.tmdb("tv", q, None, tmdb).await?,
                _ => None,
            };
            if let Some(mut a) = hit {
                if kind == "anime" {
                    a.kind = Some("anime".into());
                }
                return Ok(Some(a));
            }
        }
        Ok(match kind {
            "film" => match self.itunes(q, "movie", "600x900bb").await? {
                Some(a) => Some(a),
                None => self.wikipedia(q, year, "film").await?,
            },
            // Une série d'animation sur TVmaze, sinon AniList (hors films), sinon n'importe quelle série TVmaze
            "anime" => match self.tvmaze(q, true).await? {
                Some(a) => Some(a.kind("anime")),
                None => match self.anilist(q, "ANIME").await? {
                    Some(a) => Some(a),
                    None => self.tvmaze(q, false).await?.map(|a| a.kind("anime")),
                },
            },
            "series" => match self.tvmaze(q, false).await? {
                Some(a) => Some(a),
                None => self.anilist(q, "ANIME").await?,
            },
            // Steam, sinon Wikipédia anglais, sinon français (titres FR des jeux console : « Mario et Sonic aux Jeux olympiques »)
            // IGDB (jaquettes officielles, consoles comprises), sinon Steam, Wikipédia anglais puis français
            "game" => match self.igdb(q).await {
                Some(a) => Some(a),
                None => match self.steam(q).await? {
                Some(a) => Some(a),
                None => match self.wikipedia(q, None, "video game").await? {
                    Some(a) => Some(a),
                    None => self.wikipedia_in("fr", q, None, "jeu vidéo").await?,
                },
                },
            },
            "other" => self.steam(q).await?.map(|a| a.kind("game")),
            "software" => match self.wikipedia(q, None, "software").await? {
                Some(a) => Some(a),
                None => self.steam(q).await?,
            },
            "book" => self.anilist(q, "MANGA").await?,
            "audio" => self.itunes(q, "audiobook", "600x600bb").await?,
            _ => None,
        })
    }

    /// Jeton d'accès IGDB : celui intégré à la compilation, sinon demandé à Twitch avec le secret (~60 jours).
    async fn igdb_auth(&self, renew: bool) -> Option<String> {
        if let Some((t, until)) = self.igdb_token.lock().unwrap().clone() {
            if !renew && until > std::time::Instant::now() {
                return Some(t);
            }
        }
        // Le jeton intégré d'abord ; s'il est refusé (expiré), on en demande un nouveau avec le secret
        let built_in = IGDB_TOKEN.filter(|s| !s.is_empty());
        if !renew {
            if let Some(t) = built_in {
                *self.igdb_token.lock().unwrap() = Some((t.to_string(), std::time::Instant::now() + Duration::from_secs(365 * 86400)));
                return Some(t.to_string());
            }
        }
        let id = IGDB_ID.filter(|s| !s.is_empty())?;
        let secret = IGDB_SECRET.filter(|s| !s.is_empty())?;
        let v: Value = self
            .client
            .post("https://id.twitch.tv/oauth2/token")
            .query(&[("client_id", id), ("client_secret", secret), ("grant_type", "client_credentials")])
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        let token = v["access_token"].as_str()?.to_string();
        let ttl = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(300);
        *self.igdb_token.lock().unwrap() = Some((token.clone(), std::time::Instant::now() + Duration::from_secs(ttl)));
        Some(token)
    }

    /// Requête IGDB (jeton renouvelé une fois s'il est refusé). Une erreur → None.
    async fn igdb_query(&self, endpoint: &str, body: &str) -> Option<Vec<Value>> {
        if !igdb_enabled() {
            return None;
        }
        let id = IGDB_ID?;
        let mut renew = false;
        loop {
            let token = self.igdb_auth(renew).await?;
            let res = self.client.post(format!("https://api.igdb.com/v4/{endpoint}")).header("Client-ID", id).bearer_auth(&token).body(body.to_string()).send().await.ok()?;
            if res.status() == reqwest::StatusCode::UNAUTHORIZED && !renew {
                renew = true; // jeton expiré : on en redemande un
                continue;
            }
            return res.error_for_status().ok()?.json().await.ok();
        }
    }

    /// Jeux IGDB pour un titre (même principe que la recherche de MyPlayLog) :
    /// - la recherche plein texte d'IGDB (tolère des fautes) ;
    /// - en même temps, CHAQUE mot dans le nom ou un nom alternatif, dans n'importe quel ordre
    ///   et avec ou sans accents : titres français, ponctuation (« Zelda Ocarina »), noms raccourcis ;
    /// - rien de proche : on relâche (sans les mots de 2 lettres, puis débuts de mots pour les fautes).
    /// Renvoie (nom le plus proche, jeu) du plus proche au moins proche, les plus connus d'abord à égalité.
    async fn igdb_games(&self, q: &str, limit: usize) -> Vec<(String, Value)> {
        const FIELDS: &str = "name,alternative_names.name,cover.image_id,first_release_date,platforms.abbreviation,total_rating_count";
        let clean: String = q.chars().map(|c| if c == '"' || c == '\\' { ' ' } else { c }).collect();
        let raw: Vec<String> = clean.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).take(8).collect();
        // Chaque mot : tel quel et sans accents, dans le nom ou un nom alternatif
        let by_words = |ws: &[String]| -> Option<String> {
            if ws.is_empty() {
                return None;
            }
            let conds: Vec<String> = ws
                .iter()
                .map(|w| {
                    let mut forms = vec![w.clone(), fold(w)];
                    forms.dedup();
                    let alts: Vec<String> = forms.iter().map(|f| format!("name ~ *\"{f}\"* | alternative_names.name ~ *\"{f}\"*")).collect();
                    format!("({})", alts.join(" | "))
                })
                .collect();
            Some(format!("fields {FIELDS}; where {} & cover != null; sort total_rating_count desc; limit 25;", conds.join(" & ")))
        };
        let search = format!("search \"{}\"; fields {FIELDS}; limit 20;", clean.trim());
        let words_q = by_words(&raw).unwrap_or_default();
        let (a, b) = tokio::join!(self.igdb_query("games", &search), async {
            if words_q.is_empty() { None } else { self.igdb_query("games", &words_q).await }
        });
        let mut games: Vec<Value> = a.unwrap_or_default();
        games.extend(b.unwrap_or_default());

        let good = |games: &[Value]| games.iter().any(|g| names_of(g).iter().any(|n| similar(q, n) || contains_all(q, n)));
        // Relâcher : sans les petits mots (« of », « la »), puis seulement le début des longs mots (fautes)
        if !good(&games) {
            let long: Vec<String> = raw.iter().filter(|w| w.chars().count() > 2).cloned().collect();
            let stems: Vec<String> = long.iter().map(|w| if w.chars().count() >= 6 { w.chars().take(4).collect() } else { w.clone() }).collect();
            for ws in [long, stems] {
                if ws.len() == raw.len() && ws == raw {
                    continue;
                }
                if let Some(body) = by_words(&ws) {
                    games.extend(self.igdb_query("games", &body).await.unwrap_or_default());
                    if good(&games) {
                        break;
                    }
                }
            }
        }

        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<(String, f64, i64, Value)> = games
            .into_iter()
            .filter(|g| seen.insert(g["id"].as_i64().unwrap_or_default()))
            .map(|g| {
                let (name, score) = names_of(&g)
                    .into_iter()
                    .map(|n| {
                        let sc = closeness(q, &n) + if contains_all(q, &n) { 0.25 } else { 0.0 };
                        (n, sc)
                    })
                    .max_by(|x, y| x.1.total_cmp(&y.1))
                    .unwrap_or_default();
                let pop = g["total_rating_count"].as_i64().unwrap_or(0);
                (name, score, pop, g)
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)));
        out.into_iter().take(limit).map(|(n, _, _, g)| (n, g)).collect()
    }

    /// Jaquette IGDB du jeu le plus proche du titre (une erreur → on passe aux autres sources).
    async fn igdb(&self, q: &str) -> Option<Art> {
        self.igdb_games(q, 20).await.into_iter().find_map(|(name, g)| {
            if !similar(q, &name) && !contains_all(q, &name) {
                return None;
            }
            let cover = g["cover"]["image_id"].as_str()?;
            Some(Art::new(igdb_img(cover, "t_cover_big_2x")).fallback(igdb_img(cover, "t_cover_big")).kind("game"))
        })
    }

    async fn json(&self, req: reqwest::RequestBuilder) -> Result<Value, reqwest::Error> {
        let res = req.send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Value::Null);
        }
        res.error_for_status()?.json().await
    }

    async fn tvmaze(&self, q: &str, animation_only: bool) -> Result<Option<Art>, reqwest::Error> {
        let v = self.json(self.client.get("https://api.tvmaze.com/search/shows").query(&[("q", q)])).await?;
        let hit = v.as_array().into_iter().flatten().find_map(|r| {
            let show = &r["show"];
            let name = show["name"].as_str()?;
            let img = show["image"]["original"].as_str().or(show["image"]["medium"].as_str())?;
            if !similar(q, name) || (animation_only && show["type"].as_str() != Some("Animation")) {
                return None;
            }
            let anime = show["type"].as_str() == Some("Animation") && show["language"].as_str() == Some("Japanese");
            let art = Art::new(img);
            Some(if anime { art.kind("anime") } else { art })
        });
        Ok(hit)
    }

    async fn itunes(&self, q: &str, media: &str, size: &str) -> Result<Option<Art>, reqwest::Error> {
        // Le filtre media=movie d'iTunes ne renvoie plus rien, et la boutique FR n'a pas de films :
        // pour les films on cherche sans filtre sur la boutique US puis on trie par type.
        let country = if media == "movie" { "US" } else { "FR" };
        let mut query = vec![("term", q), ("limit", "25"), ("country", country)];
        if media != "movie" {
            query.push(("media", media));
        }
        let v = self.json(self.client.get("https://itunes.apple.com/search").query(&query)).await?;
        let hit = v["results"].as_array().into_iter().flatten().find_map(|r| {
            if media == "movie" && r["kind"].as_str() != Some("feature-movie") {
                return None;
            }
            let name = r["trackName"].as_str().or(r["collectionName"].as_str())?;
            let img = r["artworkUrl100"].as_str()?;
            similar(q, name).then(|| Art::new(img.replace("100x100bb", size)).fallback(img))
        });
        Ok(hit)
    }

    async fn tmdb(&self, kind: &str, q: &str, year: Option<u32>, key: &str) -> Result<Option<Art>, reqwest::Error> {
        let y = year.map(|y| y.to_string()).unwrap_or_default();
        let mut query = vec![("api_key", key), ("query", q), ("language", "fr-FR")];
        if !y.is_empty() && kind == "movie" {
            query.push(("year", &y));
        }
        let url = format!("https://api.themoviedb.org/3/search/{kind}");
        let v = self.json(self.client.get(url).query(&query)).await?;
        let hit = v["results"].as_array().into_iter().flatten().find_map(|r| {
            let path = r["poster_path"].as_str()?;
            Some(Art::new(format!("https://image.tmdb.org/t/p/w500{path}")))
        });
        Ok(hit)
    }

    /// Image principale de l'article Wikipédia (en anglais) : l'affiche d'un film, la jaquette d'un jeu.
    async fn wikipedia(&self, q: &str, year: Option<u32>, hint: &str) -> Result<Option<Art>, reqwest::Error> {
        self.wikipedia_in("en", q, year, hint).await
    }

    async fn wikipedia_in(&self, lang: &str, q: &str, year: Option<u32>, hint: &str) -> Result<Option<Art>, reqwest::Error> {
        let search = match year {
            Some(y) => format!("{q} {y} {hint}"),
            None => format!("{q} {hint}"),
        };
        let req = self.client.get(format!("https://{lang}.wikipedia.org/w/api.php")).query(&[
            ("action", "query"),
            ("format", "json"),
            ("generator", "search"),
            ("gsrsearch", &search),
            ("gsrlimit", "5"),
            ("prop", "pageimages"),
            ("piprop", "thumbnail"),
            ("pithumbsize", "600"),
            // les affiches ne sont pas sous licence libre : sans ça, Wikipédia ne les renvoie pas
            ("pilicense", "any"),
        ]);
        let v = self.json(req).await?;
        let mut pages: Vec<&Value> = v["query"]["pages"].as_object().into_iter().flat_map(|m| m.values()).collect();
        pages.sort_by_key(|p| p["index"].as_i64().unwrap_or(99));
        let found: Vec<(String, &str)> = pages
            .into_iter()
            .filter_map(|p| {
                let title = p["title"].as_str()?;
                let img = p["thumbnail"]["source"].as_str()?;
                Some((title.split(" (").next().unwrap_or(title).to_string(), img))
            })
            .collect();
        // Le titre exact d'abord (« Mario et Sonic aux Jeux olympiques » plutôt que « … d'hiver »)
        let exact = found.iter().find(|(t, _)| words(t) == words(q));
        let hit = exact.or_else(|| found.iter().find(|(t, _)| similar(q, t))).map(|(_, img)| Art::new(*img));
        Ok(hit)
    }

    async fn steam(&self, q: &str) -> Result<Option<Art>, reqwest::Error> {
        let req = self
            .client
            .get("https://store.steampowered.com/api/storesearch/")
            .query(&[("term", q), ("l", "french"), ("cc", "FR")]);
        let v = self.json(req).await?;
        let Some(id) = v["items"].as_array().into_iter().flatten().find_map(|r| {
            let name = r["name"].as_str()?;
            similar(q, name).then(|| r["id"].as_i64()).flatten()
        }) else {
            return Ok(None);
        };
        // Les jeux récents ont des adresses d'images « hachées » : on demande les vraies à Steam.
        if let Some(art) = self.steam_assets(id).await {
            return Ok(Some(art));
        }
        let base = format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}");
        Ok(Some(Art::new(format!("{base}/library_600x900.jpg")).fallback(format!("{base}/header.jpg"))))
    }

    async fn steam_assets(&self, id: i64) -> Option<Art> {
        let input = json!({
            "ids": [{ "appid": id }],
            "context": { "language": "french", "country_code": "FR" },
            "data_request": { "include_assets": true }
        });
        let req = self
            .client
            .get("https://api.steampowered.com/IStoreBrowseService/GetItems/v1/")
            .query(&[("input_json", input.to_string())]);
        let v = self.json(req).await.ok()?;
        let a = &v["response"]["store_items"][0]["assets"];
        let fmt = a["asset_url_format"].as_str()?;
        let url = |f: &str| format!("https://shared.akamai.steamstatic.com/store_item_assets/{}", fmt.replace("${FILENAME}", f));
        let header = a["header"].as_str().map(url);
        match a["library_capsule_2x"].as_str().or(a["library_capsule"].as_str()) {
            Some(cap) => {
                let art = Art::new(url(cap));
                Some(match header {
                    Some(h) => art.fallback(h),
                    None => art,
                })
            }
            None => header.map(Art::new),
        }
    }

    async fn anilist(&self, q: &str, kind: &str) -> Result<Option<Art>, reqwest::Error> {
        let body = json!({
            "query": "query($s:String,$t:MediaType){Page(perPage:8){media(search:$s,type:$t){format title{romaji english native} synonyms coverImage{extraLarge large}}}}",
            "variables": { "s": q, "t": kind }
        });
        let v = self.json(self.client.post("https://graphql.anilist.co").json(&body)).await?;
        let hit = v["data"]["Page"]["media"].as_array().into_iter().flatten().find_map(|m| {
            // Un torrent de série ne correspond pas à un film d'animation
            if kind == "ANIME" && m["format"].as_str() == Some("MOVIE") {
                return None;
            }
            let t = &m["title"];
            let mut names: Vec<&str> = [&t["romaji"], &t["english"], &t["native"]].iter().filter_map(|x| x.as_str()).collect();
            names.extend(m["synonyms"].as_array().into_iter().flatten().filter_map(|x| x.as_str()));
            let img = m["coverImage"]["extraLarge"].as_str().or(m["coverImage"]["large"].as_str())?;
            names.iter().any(|n| similar(q, n)).then(|| {
                let art = Art::new(img);
                if kind == "ANIME" { art.kind("anime") } else { art }
            })
        });
        Ok(hit)
    }
}

/// Image importée → JPEG réduit (600 px de large au plus) en data: URL, gardée dans le cache.
pub fn custom_image(bytes: &[u8]) -> Result<Art, String> {
    use base64::Engine;
    let img = image::load_from_memory(bytes).map_err(|_| "Image illisible (formats : JPEG, PNG, WebP)".to_string())?;
    let img = if img.width() > 600 { img.resize(600, 1800, image::imageops::FilterType::Lanczos3) } else { img };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?;
    Ok(Art::new(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(out))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test live_lookup -- --ignored --nocapture` (réseau requis)
    #[test]
    #[ignore]
    fn live_igdb() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let art = Artwork::new();
        for q in ["Time Hollow", "Mario & Sonic at the Olympic Games", "Mario et Sonic aux Jeux Olympiques d'Hiver", "Pokemon Emerald", "Pokemon Version Emeraude", "zelda ocarina", "higurshi", "Profesor Layton et l'Etrange Village", "Kingdom Hearts Birth be Sleep"] {
            println!("{q} → {:?}", rt.block_on(art.igdb(q)));
        }
        for c in rt.block_on(art.choices("game", "Mario et Sonic aux Jeux Olympiques d'Hiver", None, "")) {
            println!("  {} | {} | {} | {}", c.source, c.title, c.sub, c.url);
        }
    }

    #[test]
    #[ignore]
    fn live_lookup() {
        let a = Artwork::new();
        let cases = [
            ("anime", "JoJo's Bizarre Adventure"),
            ("series", "Black Clover"),
            ("anime", "Dreamland"),
            ("series", "Loki"),
            ("film", "Deadpool 2"),
            ("game", "Animal Crossing New Horizons"),
            ("game", "Mario Et Sonic Aux Jeux Olympiques"),
            ("other", "Meccha Chameleon"),
        ];
        tauri::async_runtime::block_on(async {
            for (k, t) in cases {
                let r = a.lookup(k, &query_of(t), None, "").await;
                println!("{k:<7} {t:<36} {:?}", r.map(|o| o.map(|x| (x.url, x.kind))));
            }
        });
    }

    #[test]
    fn similarity() {
        assert!(contains_all("zelda ocarina", "The Legend of Zelda: Ocarina of Time"));
        assert!(similar("Profeseur Layton et l'Étrange Village", "Professeur Layton et l'Etrange Village"));
        assert!(!similar("Mario Kart", "Mario Party"));
        assert!(similar("Deadpool and Wolverine", "Deadpool & Wolverine"));
        assert!(similar("JoJos Bizarre Adventure", "JoJo's Bizarre Adventure"));
        assert!(similar("Black Clover", "Black Clover"));
        assert!(!similar("Kenichi", "Kenshi"));
        assert_eq!(query_of("Adieu Eri (Fujimoto)"), "Adieu Eri");
    }
}
