//! Recherche de torrents sur les sites publics. Premier site : nyaa.si (flux RSS).

use regex::Regex;
use reqwest::Client;
use serde::Serialize;
use std::sync::LazyLock;

#[derive(Serialize, Clone, Debug)]
pub struct SearchResult {
    pub title: String,
    /// Lien du fichier .torrent (qBittorrent le télécharge lui-même)
    pub url: String,
    pub page: String,
    pub date: String,
    pub seeders: i64,
    pub leechers: i64,
    pub downloads: i64,
    pub hash: String,
    pub category: String,
    pub size: i64,
    pub trusted: bool,
    pub source: &'static str,
    /// Remplis après la recherche : titre propre, type détecté, clé d'affiche, langue
    pub clean: String,
    pub kind: String,
    pub art_key: String,
    pub year: Option<u32>,
    /// « fr », « en », « raw » ou « » (inconnu)
    pub lang: String,
    #[serde(skip)]
    pub category_id: String,
}

static RE_FR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(vostfr|vf|vff|vf2|vfq|vfi|french|truefrench|subfrench|multi|fr)\b").unwrap());
static RE_EN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(eng|english|vostEN|multi-?subs?)\b").unwrap());

/// Complète un résultat : type et titre détectés comme pour un torrent, langue des sous-titres / audio.
pub fn enrich(mut r: SearchResult) -> SearchResult {
    let raw = crate::qbit::RawTorrent { name: r.title.clone(), ..Default::default() };
    let m = crate::media::Media::from_raw(raw);
    r.clean = m.title.clone();
    r.kind = m.kind.as_str().to_string();
    r.art_key = m.group_key.clone();
    r.year = m.year;
    if r.source == "online-fix" {
        // online-fix ne publie que des jeux
        let mut m = m;
        m.set_kind(crate::media::Kind::Game);
        r.kind = m.kind.as_str().to_string();
        r.art_key = m.group_key;
    }
    r.lang = if r.source == "c411" || RE_FR.is_match(&r.title) {
        "fr".into()
    } else if r.category_id.ends_with("_4") {
        "raw".into()
    } else if r.category_id.ends_with("_2") || RE_EN.is_match(&r.title) {
        "en".into()
    } else {
        String::new()
    };
    r
}

static RE_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<item>(.*?)</item>").unwrap());

fn tag(block: &str, name: &str) -> String {
    let open = format!("<{name}");
    let Some(start) = block.find(&open) else { return String::new() };
    let Some(gt) = block[start..].find('>') else { return String::new() };
    let body_start = start + gt + 1;
    let close = format!("</{name}>");
    let Some(end) = block[body_start..].find(&close) else { return String::new() };
    decode(block[body_start..body_start + end].trim())
}

fn decode(s: &str) -> String {
    let s = s.trim_start_matches("<![CDATA[").trim_end_matches("]]>");
    let re = Regex::new(r"&#(\d+);").unwrap();
    let s = re.replace_all(s, |c: &regex::Captures| c[1].parse::<u32>().ok().and_then(char::from_u32).map(String::from).unwrap_or_default());
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// « 14.1 GiB » → octets
fn parse_size(s: &str) -> i64 {
    let mut it = s.split_whitespace();
    let n: f64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let mult = match it.next().unwrap_or("").to_lowercase().as_str() {
        "kib" | "kb" => 1024f64,
        "mib" | "mb" => 1024f64.powi(2),
        "gib" | "gb" => 1024f64.powi(3),
        "tib" | "tb" => 1024f64.powi(4),
        _ => 1.0,
    };
    (n * mult) as i64
}

pub async fn nyaa(client: &Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let res = client
        .get("https://nyaa.si/")
        .query(&[("page", "rss"), ("q", query), ("c", "0_0"), ("f", "0")])
        .send()
        .await
        .map_err(|e| format!("nyaa.si injoignable : {e}"))?;
    let xml = res.text().await.map_err(|e| e.to_string())?;
    let mut out: Vec<SearchResult> = RE_ITEM
        .captures_iter(&xml)
        .map(|c| {
            let b = &c[1];
            let n = |t: &str| tag(b, t).parse().unwrap_or(0);
            SearchResult {
                title: tag(b, "title"),
                url: tag(b, "link"),
                page: tag(b, "guid"),
                date: tag(b, "pubDate"),
                seeders: n("nyaa:seeders"),
                leechers: n("nyaa:leechers"),
                downloads: n("nyaa:downloads"),
                hash: tag(b, "nyaa:infoHash").to_lowercase(),
                category: tag(b, "nyaa:category"),
                size: parse_size(&tag(b, "nyaa:size")),
                trusted: tag(b, "nyaa:trusted") == "Yes",
                source: "nyaa",
                clean: String::new(),
                kind: String::new(),
                art_key: String::new(),
                year: None,
                lang: String::new(),
                category_id: tag(b, "nyaa:categoryId"),
            }
        })
        .filter(|r| !r.url.is_empty())
        .map(enrich)
        .collect();
    out.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    Ok(out)
}

/// Identifiants TMDB (série, film) correspondant à une recherche — pour retrouver TOUS les torrents
/// d'une série chez un tracker, même quand leur nom ne contient pas les mots cherchés.
pub async fn tmdb_ids(client: &Client, tmdb_key: &str, query: &str) -> (Option<i64>, Option<i64>) {
    if tmdb_key.is_empty() {
        return (None, None);
    }
    let first = |kind: &'static str| async move {
        let v: serde_json::Value = client
            .get(format!("https://api.themoviedb.org/3/search/{kind}"))
            .query(&[("api_key", tmdb_key), ("query", query), ("language", "fr-FR")])
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        v["results"][0]["id"].as_i64()
    };
    (first("tv").await, first("movie").await)
}

/// Recherche Torznab (C411 et tout tracker compatible Prowlarr/Jackett) : texte + série/film TMDB.
pub async fn torznab(client: &Client, base: &str, key: &str, query: &str, source: &'static str, tmdb: (Option<i64>, Option<i64>)) -> Result<Vec<SearchResult>, String> {
    if key.trim().is_empty() {
        return Err("Ajoute ta clé API C411 dans Réglages → Général → Intégrations.".into());
    }
    let mut calls: Vec<Vec<(&str, String)>> = vec![vec![("t", "search".into()), ("q", query.into())]];
    if let Some(id) = tmdb.0 {
        calls.push(vec![("t", "tvsearch".into()), ("tmdbid", id.to_string())]);
    }
    if let Some(id) = tmdb.1 {
        calls.push(vec![("t", "movie".into()), ("tmdbid", id.to_string())]);
    }
    let mut all: Vec<SearchResult> = Vec::new();
    let mut first_err = None;
    for params in calls {
        match torznab_once(client, base, key, params, source).await {
            Ok(list) => {
                for r in list {
                    if !all.iter().any(|x| x.url == r.url || (!r.hash.is_empty() && x.hash == r.hash)) {
                        all.push(r);
                    }
                }
            }
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    if all.is_empty() {
        if let Some(e) = first_err {
            return Err(e);
        }
    }
    all.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    Ok(all)
}

async fn torznab_once(client: &Client, base: &str, key: &str, mut params: Vec<(&str, String)>, source: &'static str) -> Result<Vec<SearchResult>, String> {
    let url = format!("{}/api", base.trim_end_matches('/'));
    params.push(("apikey", key.trim().to_string()));
    params.push(("limit", "100".into()));
    let res = client.get(&url).query(&params).send().await.map_err(|e| format!("{source} injoignable : {e}"))?;
    let status = res.status();
    let xml = res.text().await.map_err(|e| e.to_string())?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN || xml.contains("<error") {
        let msg = Regex::new(r#"description="([^"]*)""#).unwrap().captures(&xml).map(|c| decode(&c[1])).unwrap_or_default();
        return Err(format!("{source} a refusé la recherche{} — vérifie ta clé API.", if msg.is_empty() { String::new() } else { format!(" ({msg})") }));
    }
    let attr_re = Regex::new(r#"<torznab:attr\s+name="([^"]+)"\s+value="([^"]*)"\s*/?>"#).unwrap();
    let enclosure_re = Regex::new(r#"<enclosure[^>]*url="([^"]+)""#).unwrap();
    let mut out: Vec<SearchResult> = RE_ITEM
        .captures_iter(&xml)
        .map(|c| {
            let b = &c[1];
            let attr = |n: &str| attr_re.captures_iter(b).find(|a| &a[1] == n).map(|a| decode(&a[2])).unwrap_or_default();
            let num = |n: &str| attr(n).parse::<i64>().unwrap_or(0);
            let link = enclosure_re.captures(b).map(|e| decode(&e[1])).unwrap_or_else(|| tag(b, "link"));
            let seeders = num("seeders");
            let peers = num("peers");
            SearchResult {
                title: tag(b, "title"),
                url: link,
                page: tag(b, "comments"),
                date: tag(b, "pubDate"),
                seeders,
                leechers: (peers - seeders).max(0),
                downloads: num("grabs"),
                hash: attr("infohash").to_lowercase(),
                category: tag(b, "category"),
                size: tag(b, "size").parse().unwrap_or_else(|_| num("size")),
                trusted: false,
                source,
                clean: String::new(),
                kind: String::new(),
                art_key: String::new(),
                year: None,
                lang: String::new(),
                category_id: String::new(),
            }
        })
        .filter(|r| !r.url.is_empty())
        .map(enrich)
        .collect();
    out.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    Ok(out)
}

/// Le site online-fix est en windows-1251 (reqwest est compilé sans décodage des jeux de caractères).
fn cp1251(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0..=0x7F => b as char,
            0xC0..=0xFF => char::from_u32(0x0410 + (b - 0xC0) as u32).unwrap_or('?'),
            0xA8 => 'Ё',
            0xB8 => 'ё',
            0xA0 => ' ',
            _ => '?',
        })
        .collect()
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

static RE_OF_PAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"class="big-link" href="([^"]+)""#).unwrap());
static RE_OF_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<h2 class="title">(.*?)</h2>"#).unwrap());
static RE_OF_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s+по\s+сети\s*$").unwrap());
static RE_OF_DIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https://uploads\.online-fix\.me:2053/torrents/[^"]+"#).unwrap());
static RE_OF_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<a href="([^"]+\.torrent)">[^<]*</a>\s+(\d{2})-(\w{3})-(\d{4}) (\d{2}:\d{2})"#).unwrap());

/// Recherche sur online-fix.me (jeux réparés pour le multijoueur). Le site n'a pas d'API :
/// page de recherche → page du jeu → dossier « torrents » → fichiers .torrent (lus pour la taille et le hash).
pub async fn online_fix(client: &Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let res = client
        .get("https://online-fix.me/index.php")
        .query(&[("do", "search"), ("subaction", "search"), ("story", query)])
        .send()
        .await
        .map_err(|e| format!("online-fix.me injoignable : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("online-fix.me a répondu {}", res.status()));
    }
    let html = cp1251(&res.bytes().await.map_err(|e| e.to_string())?);
    let games: Vec<(String, String)> = html
        .split("news news-search")
        .skip(1)
        .filter_map(|b| {
            let page = RE_OF_PAGE.captures(b)?[1].to_string();
            let title = RE_OF_TITLE.captures(b).map(|c| decode(&c[1])).unwrap_or_default();
            Some((page, RE_OF_SUFFIX.replace(title.trim(), "").trim().to_string()))
        })
        .take(15)
        .collect();

    // Chaque jeu demande 3 requêtes : on les lance en parallèle
    let tasks: Vec<_> = games
        .into_iter()
        .map(|(page, title)| {
            let client = client.clone();
            tauri::async_runtime::spawn(async move { online_fix_game(&client, page, title).await })
        })
        .collect();
    let mut out = Vec::new();
    for t in tasks {
        if let Ok(list) = t.await {
            out.extend(list);
        }
    }
    Ok(out)
}

async fn online_fix_game(client: &Client, page: String, title: String) -> Vec<SearchResult> {
    let get = |url: String| {
        let client = client.clone();
        async move {
            let r = client.get(&url).header("Referer", "https://online-fix.me/").send().await.ok()?;
            r.status().is_success().then_some(())?;
            r.bytes().await.ok()
        }
    };
    let Some(html) = get(page.clone()).await else { return Vec::new() };
    let html = String::from_utf8_lossy(&html);
    let Some(dir) = RE_OF_DIR.find(&html).map(|m| m.as_str().to_string()) else { return Vec::new() };
    let dir = if dir.ends_with('/') { dir } else { format!("{dir}/") };
    let Some(listing) = get(dir.clone()).await else { return Vec::new() };
    let listing = String::from_utf8_lossy(&listing).to_string();
    let mut out = Vec::new();
    for c in RE_OF_FILE.captures_iter(&listing) {
        let url = format!("{dir}{}", &c[1]);
        let name = pct_decode(&c[1]).trim_end_matches(".torrent").to_string();
        let (mut size, mut hash) = (0, String::new());
        if let Some(bytes) = get(url.clone()).await {
            if let Some((_, files)) = crate::scan::torrent_files(&bytes) {
                size = files.iter().map(|f| f.size).sum();
            }
            hash = crate::scan::infohash(&bytes).unwrap_or_default().to_lowercase();
        }
        let r = enrich(SearchResult {
            title: if title.is_empty() { name.clone() } else { title.clone() },
            url,
            page: page.clone(),
            date: format!("{} {} {} {}:00 GMT", &c[2], &c[3], &c[4], &c[5]),
            // Le site n'indique pas les sources : -1 = inconnu
            seeders: -1,
            leechers: -1,
            downloads: 0,
            hash,
            category: "Jeux".into(),
            size,
            trusted: false,
            source: "online-fix",
            clean: String::new(),
            kind: String::new(),
            art_key: String::new(),
            year: None,
            lang: String::new(),
            category_id: String::new(),
        });
        // Titre propre du site pour l'affiche, nom du fichier pour la version
        out.push(SearchResult { title: name, ..r });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn online_fix_parsing() {
        assert_eq!(super::cp1251(b"Valheim \xEF\xEE \xF1\xE5\xF2\xE8"), "Valheim по сети");
        assert_eq!(super::RE_OF_SUFFIX.replace("Valheim по сети", ""), "Valheim");
        assert_eq!(super::pct_decode("R.E.P.O.%20v1.torrent"), "R.E.P.O. v1.torrent");
        let l = r#"<a href="Valheim.v1.0.21-OFME.torrent">Valheim.v1.0.21-OFME.torrent</a>                       04-Nov-2026 15:20     85K"#;
        let c = super::RE_OF_FILE.captures(l).unwrap();
        assert_eq!((&c[1], &c[3]), ("Valheim.v1.0.21-OFME.torrent", "Nov"));
    }

    /// Réseau : cargo test online_fix_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn online_fix_live() {
        let client = reqwest::Client::builder().user_agent("OneForAll/0.4").build().unwrap();
        let list = tauri::async_runtime::block_on(super::online_fix(&client, "valheim")).unwrap();
        for r in &list {
            println!("{} | {} | {} | {} | {} | {}", r.title, r.clean, r.kind, r.size, r.hash, r.url);
        }
        assert!(list.iter().any(|r| r.kind == "game" && r.size > 0 && !r.hash.is_empty()));
    }
    #[test]
    fn torznab_item() {
        let xml = r#"<item><title>Film 2024 MULTi 1080p</title><link>https://x/api?t=get&amp;id=1</link><size>1234</size><enclosure url="https://x/api?t=get&amp;id=1&amp;apikey=k" length="1234" type="application/x-bittorrent"/><torznab:attr name="seeders" value="12"/><torznab:attr name="peers" value="15"/><torznab:attr name="infohash" value="ABC"/></item>"#;
        let c = RE_ITEM.captures(xml).unwrap();
        let b = &c[1];
        assert_eq!(tag(b, "title"), "Film 2024 MULTi 1080p");
        assert_eq!(tag(b, "size"), "1234");
        let _ = b;
    }
    use super::*;

    #[test]
    fn parse_item() {
        let b = "<title>[X] Black Clover &amp; co &#34;1&#34;</title><link>https://nyaa.si/download/1.torrent</link><nyaa:seeders>26</nyaa:seeders><nyaa:size>14.1 GiB</nyaa:size>";
        assert_eq!(tag(b, "title"), "[X] Black Clover & co \"1\"");
        assert_eq!(tag(b, "nyaa:seeders"), "26");
        assert_eq!(parse_size("1.5 MiB"), 1572864);
    }
}
