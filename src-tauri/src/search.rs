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
            }
        })
        .filter(|r| !r.url.is_empty())
        .collect();
    out.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    Ok(out)
}

/// Recherche Torznab (C411 et tout tracker compatible Prowlarr/Jackett).
pub async fn torznab(client: &Client, base: &str, key: &str, query: &str, source: &'static str) -> Result<Vec<SearchResult>, String> {
    if key.trim().is_empty() {
        return Err("Ajoute ta clé API C411 dans Réglages → Général → Trackers.".into());
    }
    let url = format!("{}/api", base.trim_end_matches('/'));
    let res = client
        .get(&url)
        .query(&[("t", "search"), ("q", query), ("apikey", key.trim()), ("limit", "100")])
        .send()
        .await
        .map_err(|e| format!("{source} injoignable : {e}"))?;
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
            }
        })
        .filter(|r| !r.url.is_empty())
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
