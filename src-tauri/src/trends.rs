//! Tendances affichées à l'ouverture de la recherche : ce qui est populaire en ce moment
//! (TMDB pour les films et séries, AniList pour les animés et mangas, IGDB pour les jeux).
//! Mises en cache 30 minutes.

use crate::artwork::Artwork;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug)]
pub struct Trend {
    /// Titre affiché
    pub title: String,
    /// Texte à chercher (titre romaji pour un animé : c'est celui des torrents)
    pub query: String,
    pub poster: String,
    /// Année, type…
    pub sub: String,
}

const TTL: Duration = Duration::from_secs(30 * 60);
type Entry = (Instant, Box<dyn std::any::Any + Send>);
static CACHE: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

/// Résultat en cache (30 min) ou calculé (une liste vide n'est pas gardée : on réessaiera)
pub async fn cached<T, F>(key: &str, f: F) -> Vec<T>
where
    T: Clone + Send + 'static,
    F: std::future::Future<Output = Vec<T>>,
{
    if let Some((at, v)) = CACHE.lock().unwrap().get(key) {
        if at.elapsed() < TTL {
            if let Some(list) = v.downcast_ref::<Vec<T>>() {
                return list.clone();
            }
        }
    }
    let list = f.await;
    if !list.is_empty() {
        CACHE.lock().unwrap().insert(key.to_string(), (Instant::now(), Box::new(list.clone())));
    }
    list
}

pub async fn catalog(client: &Client, art: &Artwork, kind: &str, tmdb: &str) -> Vec<Trend> {
    match kind {
        "all" => tmdb_trending(client, "all", tmdb).await,
        "film" => tmdb_trending(client, "movie", tmdb).await,
        "series" => tmdb_trending(client, "tv", tmdb).await,
        "anime" => anilist_trending(client, "ANIME").await,
        "book" => anilist_trending(client, "MANGA").await,
        "game" => igdb_popular(art).await,
        _ => Vec::new(),
    }
}

async fn tmdb_trending(client: &Client, what: &str, key: &str) -> Vec<Trend> {
    if key.is_empty() {
        return Vec::new();
    }
    let url = format!("https://api.themoviedb.org/3/trending/{what}/week");
    let Ok(res) = client.get(url).query(&[("api_key", key), ("language", "fr-FR")]).send().await else { return Vec::new() };
    let Ok(v) = res.json::<Value>().await else { return Vec::new() };
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["media_type"].as_str() != Some("person"))
        .filter_map(|r| {
            let poster = r["poster_path"].as_str()?;
            let title = r["title"].as_str().or(r["name"].as_str())?.to_string();
            let year: String = r["release_date"].as_str().or(r["first_air_date"].as_str()).unwrap_or_default().chars().take(4).collect();
            let tv = r["media_type"].as_str() == Some("tv") || what == "tv";
            Some(Trend {
                query: title.clone(),
                title,
                poster: format!("https://image.tmdb.org/t/p/w342{poster}"),
                sub: [year, (if tv { "Série" } else { "Film" }).to_string()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "),
            })
        })
        .collect()
}

async fn anilist_trending(client: &Client, kind: &str) -> Vec<Trend> {
    let body = json!({
        "query": "query($t:MediaType){Page(perPage:24){media(type:$t,sort:TRENDING_DESC,isAdult:false){format seasonYear title{romaji english} coverImage{large}}}}",
        "variables": { "t": kind }
    });
    let Ok(res) = client.post("https://graphql.anilist.co").json(&body).send().await else { return Vec::new() };
    let Ok(v) = res.json::<Value>().await else { return Vec::new() };
    v["data"]["Page"]["media"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let romaji = m["title"]["romaji"].as_str()?.to_string();
            let title = m["title"]["english"].as_str().map(str::to_string).unwrap_or_else(|| romaji.clone());
            let sub = [m["seasonYear"].as_i64().map(|y| y.to_string()), m["format"].as_str().map(str::to_string)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            Some(Trend { title, query: romaji, poster: m["coverImage"]["large"].as_str()?.into(), sub })
        })
        .collect()
}

/// Jeux du moment sur IGDB : les plus joués puis les plus consultés, sortis dans l'année (sans jeux érotiques)
async fn igdb_popular(art: &Artwork) -> Vec<Trend> {
    let q = |t: u8| format!("fields game_id; where popularity_type = {t}; sort value desc; limit 300;");
    let (q3, q1) = (q(3), q(1));
    let (playing, visits) = tokio::join!(art.igdb_query("popularity_primitives", &q3), art.igdb_query("popularity_primitives", &q1));
    let mut ids: Vec<i64> = Vec::new();
    for p in playing.unwrap_or_default().iter().chain(visits.unwrap_or_default().iter()) {
        if let Some(id) = p["game_id"].as_i64() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let body = format!("fields name,cover.image_id,first_release_date,platforms.abbreviation; where id = ({list}) & cover != null & first_release_date > {} & first_release_date <= {now} & themes != (42); limit 100;", now - 365 * 86400);
    let games = art.igdb_query("games", &body).await.unwrap_or_default();
    let by_id: HashMap<i64, &Value> = games.iter().filter_map(|g| Some((g["id"].as_i64()?, g))).collect();
    ids.iter()
        .filter_map(|id| {
            let g = by_id.get(id)?;
            let name = g["name"].as_str()?.to_string();
            let cover = g["cover"]["image_id"].as_str()?;
            let year = g["first_release_date"].as_i64().map(|t| (1970 + t / 31_556_952).to_string()).unwrap_or_default();
            let plats: Vec<&str> = g["platforms"].as_array().into_iter().flatten().filter_map(|p| p["abbreviation"].as_str()).take(2).collect();
            Some(Trend {
                query: name.clone(),
                title: name,
                poster: format!("https://images.igdb.com/igdb/image/upload/t_cover_big/{cover}.jpg"),
                sub: [year, plats.join(", ")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "),
            })
        })
        .take(24)
        .collect()
}

#[cfg(test)]
mod tests {
    /// `cargo test trends_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn trends_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let c = reqwest::Client::builder().user_agent("OneForAll/0.7").build().unwrap();
        let art = crate::artwork::Artwork::new();
        let tmdb = option_env!("ONEFORALL_TMDB_KEY").unwrap_or_default();
        for k in ["all", "anime", "game"] {
            let l = rt.block_on(super::catalog(&c, &art, k, tmdb));
            println!("{k}: {} → {:?}", l.len(), l.iter().take(4).map(|t| format!("{} ({})", t.title, t.sub)).collect::<Vec<_>>());
        }
    }
}
