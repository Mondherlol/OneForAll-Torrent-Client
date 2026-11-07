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

#[cfg(test)]
mod tests {
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
