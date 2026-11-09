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

/// Mots en minuscules, sans ponctuation.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
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
    let common = a.iter().filter(|w| b.contains(w)).count() as f64;
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

    /// Jaquette IGDB du jeu le plus proche du titre (une erreur → on passe aux autres sources).
    async fn igdb(&self, q: &str) -> Option<Art> {
        if !igdb_enabled() {
            return None;
        }
        let id = IGDB_ID?;
        let body = format!("search \"{}\"; fields name,alternative_names.name,cover.image_id,artworks.image_id; limit 10;", q.replace('"', " "));
        let mut renew = false;
        let games: Vec<Value> = loop {
            let token = self.igdb_auth(renew).await?;
            let res = self.client.post("https://api.igdb.com/v4/games").header("Client-ID", id).bearer_auth(&token).body(body.clone()).send().await.ok()?;
            if res.status() == reqwest::StatusCode::UNAUTHORIZED && !renew {
                renew = true; // jeton expiré : on en redemande un
                continue;
            }
            break res.error_for_status().ok()?.json().await.ok()?;
        };
        let img = |id: &str, size: &str| format!("https://images.igdb.com/igdb/image/upload/{size}/{id}.jpg");
        games.iter().find_map(|g| {
            let names = std::iter::once(g["name"].as_str()?).chain(g["alternative_names"].as_array().into_iter().flatten().filter_map(|n| n["name"].as_str()));
            if !names.into_iter().any(|n| similar(q, n)) {
                return None;
            }
            let cover = g["cover"]["image_id"].as_str()?;
            let mut a = Art::new(img(cover, "t_cover_big_2x")).fallback(img(cover, "t_cover_big"));
            a.kind = Some("game".into());
            Some(a)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test live_lookup -- --ignored --nocapture` (réseau requis)
    #[test]
    #[ignore]
    fn live_igdb() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let art = Artwork::new();
        for q in ["Time Hollow", "Mario & Sonic at the Olympic Games", "Pokemon Emerald"] {
            println!("{q} → {:?}", rt.block_on(art.igdb(q)));
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
        assert!(similar("Deadpool and Wolverine", "Deadpool & Wolverine"));
        assert!(similar("JoJos Bizarre Adventure", "JoJo's Bizarre Adventure"));
        assert!(similar("Black Clover", "Black Clover"));
        assert!(!similar("Kenichi", "Kenshi"));
        assert_eq!(query_of("Adieu Eri (Fujimoto)"), "Adieu Eri");
    }
}
