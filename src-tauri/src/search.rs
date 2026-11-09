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
    /// Jaquette fournie par le site (sites de ROM)
    pub thumb: String,
    /// Console (sites de ROM) : « Nintendo DS »
    pub platform: String,
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
    if matches!(r.source, "online-fix" | "fitgirl" | "nxbrew" | "romsfun" | "wowroms") {
        // Sites qui ne publient que des jeux
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
                thumb: String::new(),
                platform: String::new(),
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
                thumb: String::new(),
                platform: String::new(),
            }
        })
        .filter(|r| !r.url.is_empty())
        .map(enrich)
        .collect();
    out.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    Ok(out)
}

fn blank(source: &'static str) -> SearchResult {
    SearchResult {
        title: String::new(),
        url: String::new(),
        page: String::new(),
        date: String::new(),
        seeders: -1,
        leechers: -1,
        downloads: 0,
        hash: String::new(),
        category: "Jeux".into(),
        size: 0,
        trusted: false,
        source,
        clean: String::new(),
        kind: String::new(),
        art_key: String::new(),
        year: None,
        lang: String::new(),
        category_id: String::new(),
        thumb: String::new(),
        platform: String::new(),
    }
}

/// Le titre affiché garde la version (« Hollow Knight – v1.5 + DLC »), l'affiche se cherche sur le nom seul.
fn enrich_game(mut r: SearchResult, art_name: &str) -> SearchResult {
    let full = std::mem::replace(&mut r.title, art_name.to_string());
    let mut r = enrich(r);
    r.title = full;
    r
}

static RE_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
fn strip_tags(s: &str) -> String {
    decode(RE_TAGS.replace_all(s, " ").split_whitespace().collect::<Vec<_>>().join(" ").trim())
}

static RE_MAGNET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"magnet:\?xt=urn:btih:[^"'<\s]+"#).unwrap());
static RE_BTIH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)btih:([0-9a-z]+)").unwrap());
static RE_FG_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)Repack Size:\s*(?:<[^>]+>\s*)*(?:from\s+)?([\d.,]+)\s*([KMGT]B)").unwrap());

/// FitGirl Repacks (jeux PC compressés) : API WordPress du site, magnet dans le contenu de chaque article.
pub async fn fitgirl(client: &Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let res = client
        .get("https://fitgirl-repacks.site/wp-json/wp/v2/posts")
        .query(&[("search", query), ("per_page", "20"), ("_fields", "title,link,date,content")])
        .send()
        .await
        .map_err(|e| format!("fitgirl-repacks.site injoignable : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("FitGirl a répondu {}", res.status()));
    }
    let posts: Vec<serde_json::Value> = res.json().await.map_err(|e| format!("Réponse FitGirl illisible : {e}"))?;
    Ok(posts
        .iter()
        .filter_map(|p| {
            let html = p["content"]["rendered"].as_str()?;
            let magnet = decode(RE_MAGNET.find(html)?.as_str());
            let title = strip_tags(p["title"]["rendered"].as_str().unwrap_or_default());
            let size = RE_FG_SIZE.captures(html).map(|c| parse_size(&format!("{} {}", c[1].replace(',', "."), &c[2]))).unwrap_or(0);
            let hash = RE_BTIH.captures(&magnet).map(|c| c[1].to_lowercase()).filter(|h| h.len() == 40).unwrap_or_default();
            let name = title.split([',', '–', '+', '(']).next().unwrap_or(&title).trim().to_string();
            let r = SearchResult {
                title,
                url: magnet,
                page: p["link"].as_str().unwrap_or_default().to_string(),
                date: p["date"].as_str().unwrap_or_default().to_string(),
                hash,
                size,
                trusted: true,
                ..blank("fitgirl")
            };
            Some(enrich_game(r, &name))
        })
        .collect())
}

static RE_NX_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<h2 class="post-title">\s*<a href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap());
static RE_NX_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<div class="post-date">\s*(?:<i[^>]*></i>)?\s*([^<]+)<"#).unwrap());
static RE_NX_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Title Size:\s*([\d.,]+)\s*([KMGT]B)").unwrap());
static RE_NX_PLATFORM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*(\[[^\]]*\]|\bSwitch 2\b|\bNSP\b|\bXCI\b|\+ *Update.*$|\+ *DLC.*$)").unwrap());

/// NXBrew (jeux Switch) : les fichiers sont sur des hébergeurs (1fichier, DataNodes…), derrière des liens ouo.io.
/// La recherche donne les jeux ; les liens s'affichent dans l'aperçu (voir `nxbrew_links`).
pub async fn nxbrew(client: &Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let res = client.get("https://nxbrew.net/").query(&[("s", query)]).send().await.map_err(|e| format!("nxbrew.net injoignable : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("nxbrew.net a répondu {} (le site bloque parfois les connexions automatiques)", res.status()));
    }
    let html = res.text().await.map_err(|e| e.to_string())?;
    let games: Vec<(String, String, String)> = html
        .split("<article")
        .skip(1)
        .filter_map(|a| {
            let t = RE_NX_TITLE.captures(a)?;
            let date = RE_NX_DATE.captures(a).map(|c| c[1].trim().to_string()).unwrap_or_default();
            Some((t[1].to_string(), strip_tags(&t[2]), date))
        })
        .take(15)
        .collect();
    // La taille n'est que sur la page du jeu : on les lit en parallèle
    let tasks: Vec<_> = games
        .into_iter()
        .map(|(page, title, date)| {
            let client = client.clone();
            tauri::async_runtime::spawn(async move {
                let size = match client.get(&page).send().await {
                    Ok(r) => r.text().await.ok().and_then(|h| RE_NX_SIZE.captures(&h).map(|c| parse_size(&format!("{} {}", c[1].replace(',', "."), &c[2])))),
                    Err(_) => None,
                };
                let name = RE_NX_PLATFORM.replace_all(&title, "").trim().to_string();
                let r = SearchResult { title, url: page.clone(), page, date, size: size.unwrap_or(0), ..blank("nxbrew") };
                enrich_game(r, &name)
            })
        })
        .collect();
    let mut out = Vec::new();
    for t in tasks {
        if let Ok(r) = t.await {
            out.push(r);
        }
    }
    Ok(out)
}

#[derive(Serialize, Debug)]
pub struct NxLink {
    pub label: String,
    pub url: String,
}
#[derive(Serialize, Debug)]
pub struct NxHost {
    pub host: String,
    pub links: Vec<NxLink>,
}
#[derive(Serialize, Debug)]
pub struct NxSection {
    /// « Base Game ROM (21.5 GB) », « Update v1.2 », nom du DLC…
    pub label: String,
    pub hosts: Vec<NxHost>,
}

const DL_HOSTS: &[&str] = &["ouo.io", "ouo.press", "1fichier.com", "datanodes.to", "multiup.", "filekeeper", "gofile.io", "buzzheavier", "mediafire.com", "pixeldrain"];
static RE_P: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<p[^>]*>(.*?)</p>").unwrap());
static RE_STRONG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<strong>(.*?)</strong>").unwrap());
static RE_A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<a [^>]*href="(https?://[^"]+)"[^>]*>(.*?)</a>"#).unwrap());

/// Liens de téléchargement d'une page NXBrew, par partie (jeu, mise à jour, DLC) et par hébergeur.
pub fn nxbrew_sections(html: &str) -> Vec<NxSection> {
    let mut out = Vec::new();
    for block in html.split(r#"class="wp-block-columns "#).skip(1) {
        let block = block.split(r#"class="wp-block-columns "#).next().unwrap_or(block);
        let Some(label) = RE_STRONG.captures(block).map(|c| strip_tags(&c[1])) else { continue };
        if !["Base Game", "Update", "DLC", "ROM"].iter().any(|k| label.contains(k)) {
            continue;
        }
        let mut hosts: Vec<NxHost> = Vec::new();
        for p in RE_P.captures_iter(block) {
            let p = &p[1];
            let links: Vec<(String, String)> = RE_A
                .captures_iter(p)
                .map(|a| (decode(&a[1]), strip_tags(&a[2])))
                // Seulement les liens de téléchargement (pas les tags, articles liés, GitHub…)
                .filter(|(u, _)| DL_HOSTS.iter().any(|h| u.contains(h)))
                .collect();
            if links.is_empty() {
                continue;
            }
            match RE_STRONG.captures(p).map(|c| strip_tags(&c[1])) {
                // « <strong>1Fichier</strong> Part1, Part2 »
                Some(host) if host != label => hosts.push(NxHost { host, links: links.into_iter().map(|(url, label)| NxLink { label, url }).collect() }),
                Some(_) => {}
                // « <a>DataNodes</a> <a>1Fichier</a> » : un lien par hébergeur
                None => {
                    for (url, host) in links {
                        hosts.push(NxHost { host, links: vec![NxLink { label: "Télécharger".into(), url }] });
                    }
                }
            }
        }
        if !hosts.is_empty() {
            out.push(NxSection { label, hosts });
        }
    }
    out
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

pub fn pct_decode(s: &str) -> String {
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
            thumb: String::new(),
            platform: String::new(),
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

    /// Réseau : cargo test games_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn games_live() {
        let client = reqwest::Client::builder().user_agent("OneForAll/0.4").build().unwrap();
        let fg = tauri::async_runtime::block_on(super::fitgirl(&client, "hollow knight")).unwrap();
        for r in fg.iter().take(4) {
            println!("FG {} | {} | {} | {} | {}", r.title, r.clean, r.kind, r.size, r.hash);
        }
        assert!(fg.iter().any(|r| r.url.starts_with("magnet:") && r.kind == "game"));
        let nx = tauri::async_runtime::block_on(super::nxbrew(&client, "mario kart")).unwrap();
        for r in nx.iter().take(4) {
            println!("NX {} | {} | {} | {}", r.title, r.clean, r.size, r.page);
        }
        assert!(!nx.is_empty());
        let html = tauri::async_runtime::block_on(async { client.get(&nx[0].page).send().await.unwrap().text().await.unwrap() });
        for s in super::nxbrew_sections(&html) {
            println!("  [{}] {}", s.label, s.hosts.iter().map(|h| format!("{}×{}", h.host, h.links.len())).collect::<Vec<_>>().join(", "));
        }
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

// ---------- sites de ROM ----------

static RE_RF_CARD_HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="(https://romsfun\.com/roms/[^"]+\.html)""#).unwrap());
static RE_IMG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<img[^>]*?(?:data-src|src)="([^"]+)"[^>]*?alt="([^"]*)""#).unwrap());
static RE_H3_A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<h3[^>]*>\s*<a[^>]*>(.*?)</a>").unwrap());
static RE_RF_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([\d.]+)\s*([KMGT])B?\s*</span>").unwrap());

fn short_size(n: &str, unit: &str) -> i64 {
    parse_size(&format!("{n} {unit}B"))
}

/// RomsFun : page de recherche WordPress, une carte par jeu (jaquette, console, taille).
pub async fn romsfun(client: &Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let html = client
        .get("https://romsfun.com/")
        .query(&[("s", query)])
        .send()
        .await
        .map_err(|e| format!("RomsFun injoignable : {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(romsfun_parse(&html))
}

pub fn romsfun_parse(html: &str) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for card in html.split(r#"<div class="bg-white rounded-xl p-3 flex gap-4"#).skip(1) {
        let Some(page) = RE_RF_CARD_HREF.captures(card).map(|c| c[1].to_string()) else { continue };
        let Some(title) = RE_H3_A.captures(card).map(|c| strip_tags(&c[1])) else { continue };
        let imgs: Vec<(String, String)> = RE_IMG.captures_iter(card).map(|c| (c[1].to_string(), decode(&c[2]))).collect();
        let mut r = blank("romsfun");
        r.title = title.clone();
        r.url = page.clone();
        r.page = page;
        r.thumb = imgs.first().map(|i| i.0.clone()).unwrap_or_default();
        r.platform = imgs.get(1).map(|i| i.1.clone()).unwrap_or_default();
        r.size = RE_RF_SIZE.captures_iter(card).last().map(|c| short_size(&c[1], &c[2])).unwrap_or(0);
        out.push(enrich_game(r, &title));
    }
    out
}

#[derive(Serialize, Debug)]
pub struct RomVersion {
    pub label: String,
    pub size: i64,
    /// Page de téléchargement de cette version
    pub page: String,
}

static RE_RF_DL_LIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="(https://romsfun\.com/download/[a-z0-9-]+)""#).unwrap());
static RE_TR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<tr[^>]*>(.*?)</tr>").unwrap());
static RE_RF_VER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)href="(https://romsfun\.com/download/[a-z0-9-]+/\d+)"[^>]*>(.*?)</a>"#).unwrap());
static RE_RF_TD_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([\d.]+)\s*([KMGT])B?\s*</td>").unwrap());

async fn get_html(client: &Client, url: &str) -> Result<String, String> {
    let res = client.get(url).send().await.map_err(|e| format!("Site injoignable : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Le site a répondu {}", res.status()));
    }
    res.text().await.map_err(|e| e.to_string())
}

/// Versions d'un jeu RomsFun (Europe, USA…) : page du jeu → page « Download » → tableau.
pub async fn romsfun_versions(client: &Client, page: &str) -> Result<Vec<RomVersion>, String> {
    let html = get_html(client, page).await?;
    let list = RE_RF_DL_LIST.captures(&html).map(|c| c[1].to_string()).ok_or("Pas de lien de téléchargement sur cette page")?;
    let html = get_html(client, &list).await?;
    let out: Vec<RomVersion> = RE_TR
        .captures_iter(&html)
        .filter_map(|tr| {
            let row = &tr[1];
            let v = RE_RF_VER.captures(row)?;
            Some(RomVersion {
                page: v[1].to_string(),
                label: strip_tags(&v[2]),
                size: RE_RF_TD_SIZE.captures(row).map(|c| short_size(&c[1], &c[2])).unwrap_or(0),
            })
        })
        .collect();
    if out.is_empty() {
        return Err("Aucune version trouvée sur RomsFun".into());
    }
    Ok(out)
}

/// Lien direct (signé, valable peu de temps) d'une version. Comme le site : la page de la version
/// demande son lien à admin-ajax.php (la page elle-même est en cache, son lien est souvent périmé).
pub async fn romsfun_link(client: &Client, page: &str) -> Result<String, String> {
    let res = client
        .post("https://romsfun.com/wp-admin/admin-ajax.php")
        .header("Referer", page)
        .header("X-Requested-With", "XMLHttpRequest")
        .form(&[("action", "k_get_download")])
        .send()
        .await
        .map_err(|e| format!("RomsFun injoignable : {e}"))?;
    let v: serde_json::Value = res.json().await.map_err(|e| format!("Réponse illisible de RomsFun : {e}"))?;
    if let Some(url) = v["data"]["download_url"].as_str().filter(|u| u.starts_with("http")) {
        return Ok(url.to_string());
    }
    let msg = v["data"]["message"].as_str().unwrap_or("lien introuvable");
    Err(format!("RomsFun : {msg} (limite de téléchargements du site atteinte ?)"))
}

static RE_WW_HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="(/fr/roms/[^"/]+/[^"/]+/\d+\.html)""#).unwrap());
static RE_WW_ALPHA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-alpha="([^"]*)""#).unwrap());
static RE_WW_CONSOLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)class="col-title[^"]*"[^>]*>(.*?)</a>"#).unwrap());
static RE_WW_IMG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-src="([^"]+)""#).unwrap());

/// WowRoms : liste de recherche. Le téléchargement passe par un jeton calculé dans la page :
/// on ouvre la page dans le navigateur (pas de contournement).
pub async fn wowroms(client: &Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let html = client
        .get("https://wowroms.com/fr/roms/list")
        .query(&[("search", query)])
        .send()
        .await
        .map_err(|e| format!("WowRoms injoignable : {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(wowroms_parse(&html))
}

pub fn wowroms_parse(html: &str) -> Vec<SearchResult> {
    // Les résultats sont après le champ de recherche (avant : « Top téléchargement »)
    let body = html.split("ulromlist").nth(1).unwrap_or("");
    let mut out = Vec::new();
    for el in body.split(r#"<li class="element"#).skip(1) {
        let Some(href) = RE_WW_HREF.captures(el).map(|c| c[1].to_string()) else { continue };
        let Some(title) = RE_WW_ALPHA.captures(el).map(|c| decode(&c[1])) else { continue };
        let page = format!("https://wowroms.com{href}");
        let mut r = blank("wowroms");
        r.title = title.clone();
        r.url = page.clone();
        r.page = page;
        r.thumb = RE_WW_IMG.captures(el).map(|c| c[1].to_string()).unwrap_or_default();
        r.platform = RE_WW_CONSOLE.captures(el).map(|c| strip_tags(&c[1].replace("&nbsp;", " "))).unwrap_or_default();
        out.push(enrich_game(r, title.split(" (").next().unwrap_or(&title)));
    }
    out
}

#[cfg(test)]
mod rom_live {
    #[test]
    #[ignore]
    fn rom_sites() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let c = reqwest::Client::builder().user_agent("OneForAll/0.6").build().unwrap();
            let rf = super::romsfun(&c, "time hollow").await.unwrap();
            for r in rf.iter().take(4) {
                println!("RF {} | {} | {} | {} | {} | {}", r.title, r.platform, r.size, r.kind, r.page, r.thumb);
            }
            let v = super::romsfun_versions(&c, &rf[0].page).await.unwrap();
            for x in &v {
                println!("  V {} | {} | {}", x.label, x.size, x.page);
            }
            println!("  LINK {}", super::romsfun_link(&c, &v[0].page).await.unwrap());
            let ww = super::wowroms(&c, "time hollow").await.unwrap();
            for r in ww.iter().take(4) {
                println!("WW {} | {} | {} | {}", r.title, r.platform, r.page, r.thumb);
            }
        });
    }
}

// ---------- populaires de chaque site (écran d'accueil de la recherche) ----------

/// Ce qui est populaire en ce moment sur un site. `genre` : genre choisi dans la recherche.
/// C411 et Nyaa : de vrais torrents ; les autres : des jeux (titre, jaquette, page) à chercher au clic.
pub async fn popular(client: &Client, source: &str, genre: &str, c411_key: &str) -> Result<Vec<SearchResult>, String> {
    match source {
        "c411" => c411_popular(client, genre, c411_key).await,
        "nyaa" => nyaa_popular(client, genre).await,
        "fitgirl" => fitgirl_popular(client).await,
        "online-fix" => online_fix_popular(client).await,
        "romsfun" => romsfun_popular(client).await,
        _ => Ok(Vec::new()),
    }
}

/// C411 : les sorties récentes (≈ 300 dernières du genre) les plus partagées
async fn c411_popular(client: &Client, genre: &str, key: &str) -> Result<Vec<SearchResult>, String> {
    if key.trim().is_empty() {
        return Err("Ajoute ta clé API C411 dans Réglages → Général → Intégrations.".into());
    }
    let cat = match genre {
        "film" => "2000",
        "series" | "anime" => "5000",
        "audio" => "3000",
        "software" | "game" => "4000",
        "book" => "7000",
        _ => "",
    };
    let page = |offset: u32| {
        let mut p: Vec<(&str, String)> = vec![("t", "search".into()), ("q", String::new()), ("offset", offset.to_string())];
        if !cat.is_empty() {
            p.push(("cat", cat.into()));
        }
        torznab_once(client, "https://c411.org", key, p, "c411")
    };
    let (a, b, c) = tokio::join!(page(0), page(100), page(200));
    let mut all: Vec<SearchResult> = Vec::new();
    let mut err = None;
    for r in [a, b, c] {
        match r {
            Ok(list) => all.extend(list),
            Err(e) => err = Some(e),
        }
    }
    if all.is_empty() {
        return Err(err.unwrap_or_default());
    }
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert(r.url.clone()) && (genre == "all" || genre == r.kind || (genre == "series" && r.kind == "anime") || !matches!(genre, "film" | "series" | "anime")));
    all.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    all.truncate(24);
    Ok(all)
}

static RE_NY_ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<a href="/view/(\d+)" title="([^"]+)">.*?href="(magnet:[^"]+)".*?<td class="text-center">([^<]+)</td>\s*<td class="text-center" data-timestamp="(\d+)">.*?<td class="text-center">(\d+)</td>\s*<td class="text-center">(\d+)</td>\s*<td class="text-center">(\d+)</td>"#).unwrap());

/// Nyaa : les torrents les plus partagés de la catégorie
async fn nyaa_popular(client: &Client, genre: &str) -> Result<Vec<SearchResult>, String> {
    let cat = match genre {
        "book" => "3_0",
        "audio" => "2_0",
        _ => "1_0",
    };
    let html = client
        .get("https://nyaa.si/")
        .query(&[("c", cat), ("s", "seeders"), ("o", "desc")])
        .send()
        .await
        .map_err(|e| format!("nyaa.si injoignable : {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(nyaa_parse_html(&html))
}

pub fn nyaa_parse_html(html: &str) -> Vec<SearchResult> {
    html.split("<tr class=\"")
        .skip(1)
        .filter_map(|row| {
            let c = RE_NY_ROW.captures(row)?;
            let magnet = decode(&c[3]);
            let mut r = blank("nyaa");
            r.title = decode(&c[2]);
            r.url = format!("https://nyaa.si/download/{}.torrent", &c[1]);
            r.page = format!("https://nyaa.si/view/{}", &c[1]);
            r.hash = RE_BTIH.captures(&magnet).map(|h| h[1].to_lowercase()).unwrap_or_default();
            r.size = parse_size(c[4].trim());
            r.date = c[5].parse::<i64>().ok().and_then(|t| chrono_rfc2822(t)).unwrap_or_default();
            r.seeders = c[6].parse().unwrap_or(0);
            r.leechers = c[7].parse().unwrap_or(0);
            r.downloads = c[8].parse().unwrap_or(0);
            r.category = String::new();
            Some(enrich(r))
        })
        .take(100)
        .collect()
}

/// Date RFC 2822 (comme les flux RSS) d'un horodatage Unix
fn chrono_rfc2822(t: i64) -> Option<String> {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // Jour civil (algorithme de Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        DAYS[days.rem_euclid(7) as usize],
        d,
        MONTHS[(m - 1) as usize],
        y,
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    ))
}

static RE_FG_POP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"widget-grid-view-image"><a href="([^"]+)" title="([^"]+)"[^>]*><img[^>]*?src="([^"]+)""#).unwrap());

static RE_ADULT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)18\+|\bsex|hentai|nsfw|milf|\bporn|erotic").unwrap());

/// FitGirl : page « Popular repacks » du site (sans les jeux pour adultes)
async fn fitgirl_popular(client: &Client) -> Result<Vec<SearchResult>, String> {
    let html = get_html(client, "https://fitgirl-repacks.site/popular-repacks/").await?;
    let mut seen = std::collections::HashSet::new();
    Ok(RE_FG_POP
        .captures_iter(&html)
        .filter(|c| seen.insert(c[1].to_string()) && !RE_ADULT.is_match(&c[2]))
        .map(|c| {
            let title = decode(&c[2]);
            let name = title.split([',', '–', '(']).next().unwrap_or(&title).split(" - ").next().unwrap_or(&title).trim().to_string();
            let mut r = blank("fitgirl");
            r.page = c[1].to_string();
            r.thumb = decode(&c[3].replace("&#038;", "&"));
            r.clean = name.clone();
            r.title = title;
            let r = enrich_game(r, &name);
            SearchResult { clean: name, ..r }
        })
        .take(24)
        .collect())
}

static RE_OF_POP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)<a href="(/games/[^"]+)" title="([^"]+)">\s*(?:<!--.*?-->\s*)?<img src="([^"]+)""#).unwrap());

/// Online-Fix : bandeau « Популярное » (populaire) de la page d'accueil
async fn online_fix_popular(client: &Client) -> Result<Vec<SearchResult>, String> {
    let res = client.get("https://online-fix.me/").send().await.map_err(|e| format!("online-fix.me injoignable : {e}"))?;
    let html = cp1251(&res.bytes().await.map_err(|e| e.to_string())?);
    let start = html.find("Популярное").ok_or("Pas de jeux populaires sur online-fix.me")?;
    let block = &html[start..];
    let block = &block[..block.find("<div class=\"news").unwrap_or(block.len())];
    Ok(RE_OF_POP
        .captures_iter(block)
        .map(|c| {
            let name = RE_OF_SUFFIX.replace(decode(&c[2]).trim(), "").trim().to_string();
            let mut r = blank("online-fix");
            r.page = format!("https://online-fix.me{}", &c[1]);
            r.thumb = c[3].to_string();
            r.title = name.clone();
            let r = enrich_game(r, &name);
            SearchResult { clean: name, ..r }
        })
        .take(24)
        .collect())
}

/// RomsFun : section « Popular ROM » de la page d'accueil
async fn romsfun_popular(client: &Client) -> Result<Vec<SearchResult>, String> {
    let html = get_html(client, "https://romsfun.com/").await?;
    let start = html.find("Popular ROM").ok_or("Pas de section populaire sur RomsFun")?;
    let block = &html[start..];
    let block = &block[..block.find("Fresh upload").unwrap_or(block.len())];
    Ok(romsfun_parse(block))
}

#[cfg(test)]
mod popular_live {
    /// `cargo test popular_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sites() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let c = reqwest::Client::builder().user_agent("OneForAll/0.7").build().unwrap();
        for s in ["nyaa", "fitgirl", "online-fix", "romsfun"] {
            match rt.block_on(super::popular(&c, s, "all", "")) {
                Ok(l) => println!("{s}: {} → {:?}", l.len(), l.iter().take(3).map(|r| format!("{} | {} | {} | {}", r.clean, r.seeders, r.thumb.chars().take(40).collect::<String>(), r.url.chars().take(40).collect::<String>())).collect::<Vec<_>>()),
                Err(e) => println!("{s}: ERR {e}"),
            }
        }
    }
}
