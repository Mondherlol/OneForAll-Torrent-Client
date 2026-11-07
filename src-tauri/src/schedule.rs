//! Calendrier de diffusion : épisodes sortis et prochain épisode.
//! Animés → AniList (entrée « en cours de diffusion », épisodes numérotés dans la saison),
//! séries (ou animé introuvable) → TVmaze (fiche en cours de diffusion de préférence).

use crate::artwork::{similar, strip_season};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug)]
pub struct Ep {
    /// 0 = « saison en cours » (AniList numérote les épisodes dans l'entrée)
    pub season: i64,
    pub number: i64,
    pub name: String,
    /// Date de diffusion : `at` (secondes Unix) ou `airstamp` (ISO 8601)
    pub at: i64,
    pub airstamp: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Schedule {
    pub show: String,
    pub status: String,
    pub source: &'static str,
    pub next: Option<Ep>,
    pub episodes: Vec<Ep>,
}

pub struct Schedules {
    client: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Option<Schedule>)>>,
}

fn tvmaze_ep(v: &Value) -> Option<Ep> {
    Some(Ep {
        season: v["season"].as_i64()?,
        number: v["number"].as_i64()?,
        name: v["name"].as_str().unwrap_or_default().to_string(),
        at: 0,
        airstamp: v["airstamp"].as_str().unwrap_or_default().to_string(),
    })
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// « STEEL BALL RUN JoJo's Bizarre Adventure 2nd STAGE » contient « JoJo's Bizarre Adventure »
fn contains_title(q: &str, found: &str) -> bool {
    let q = norm(q);
    q.len() >= 8 && norm(found).contains(&q)
}

impl Schedules {
    pub fn new() -> Self {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(12)).user_agent("OneForAll/0.7").build().expect("client HTTP");
        Self { client, cache: Mutex::new(HashMap::new()) }
    }

    /// Calendrier d'une série (cache 6 h ; « rien trouvé » 1 h).
    pub async fn get(&self, titles: &[String], kind: &str) -> Option<Schedule> {
        let title = titles.first()?.as_str();
        let key = format!("{kind}:{}", title.to_lowercase());
        if let Some((t, v)) = self.cache.lock().unwrap().get(&key) {
            let ttl = if v.is_some() { 6 * 3600 } else { 3600 };
            if t.elapsed() < Duration::from_secs(ttl) {
                return v.clone();
            }
        }
        // Toutes les variantes du titre (« JoJos » / « JoJo's »…), puis sans mention de saison
        let mut queries: Vec<String> = Vec::new();
        for t in titles {
            for q in [t.clone(), strip_season(t)] {
                if !q.is_empty() && !queries.contains(&q) {
                    queries.push(q);
                }
            }
        }
        // Les deux sources (dans l'ordre le plus probable), et on garde la meilleure :
        // un prochain épisode annoncé > une diffusion en cours > le reste.
        // (ex. JoJo : TVmaze le dit « terminé », AniList a Steel Ball Run en cours)
        let order: [&str; 2] = if kind == "anime" { ["anilist", "tvmaze"] } else { ["tvmaze", "anilist"] };
        let score = |s: &Schedule| s.next.is_some() as i32 * 2 + matches!(s.status.as_str(), "Running" | "RELEASING") as i32;
        let mut best: Option<Schedule> = None;
        let mut any_ok = false;
        'outer: for src in order {
            for q in queries.iter().map(String::as_str) {
                let r = if src == "anilist" { self.anilist(q).await } else { self.tvmaze(q, kind == "anime").await };
                let Ok(r) = r else { continue };
                any_ok = true;
                if let Some(s) = r {
                    if best.as_ref().is_none_or(|b| score(&s) > score(b)) {
                        best = Some(s);
                    }
                    if best.as_ref().is_some_and(|b| score(b) >= 2) {
                        break 'outer; // prochain épisode trouvé : inutile de chercher plus
                    }
                    break; // source suivante
                }
            }
        }
        if !any_ok {
            return None; // erreur réseau : pas de cache, on réessaiera
        }
        let found = best;
        self.cache.lock().unwrap().insert(key, (Instant::now(), found.clone()));
        found
    }

    async fn anilist(&self, q: &str) -> Result<Option<Schedule>, reqwest::Error> {
        let body = json!({
            "query": "query($s:String){Page(perPage:12){media(search:$s,type:ANIME,sort:SEARCH_MATCH){status format title{romaji english} synonyms nextAiringEpisode{episode airingAt} airingSchedule(perPage:100){nodes{episode airingAt}}}}}",
            "variables": { "s": q }
        });
        let v: Value = self.client.post("https://graphql.anilist.co").json(&body).send().await?.error_for_status()?.json().await?;
        let media = v["data"]["Page"]["media"].as_array().cloned().unwrap_or_default();
        // Parmi les entrées qui correspondent au titre : celle en cours de diffusion, sinon celle qui va sortir
        let matches = |m: &&Value| {
            let t = &m["title"];
            let mut names: Vec<&str> = [&t["romaji"], &t["english"]].iter().filter_map(|x| x.as_str()).collect();
            names.extend(m["synonyms"].as_array().into_iter().flatten().filter_map(|x| x.as_str()));
            names.iter().any(|n| similar(q, n) || contains_title(q, n)) && m["format"].as_str() != Some("MOVIE")
        };
        let pick = media
            .iter()
            .filter(matches)
            .find(|m| m["status"].as_str() == Some("RELEASING"))
            .or_else(|| media.iter().filter(matches).find(|m| m["status"].as_str() == Some("NOT_YET_RELEASED")));
        let Some(m) = pick else { return Ok(None) };
        let ep = |n: &Value| -> Option<Ep> {
            Some(Ep { season: 0, number: n["episode"].as_i64()?, name: String::new(), at: n["airingAt"].as_i64()?, airstamp: String::new() })
        };
        let episodes = m["airingSchedule"]["nodes"].as_array().into_iter().flatten().filter_map(ep).collect();
        let name = m["title"]["english"].as_str().or(m["title"]["romaji"].as_str()).unwrap_or(q).to_string();
        Ok(Some(Schedule { show: name, status: m["status"].as_str().unwrap_or_default().to_string(), source: "anilist", next: ep(&m["nextAiringEpisode"]), episodes }))
    }

    async fn tvmaze(&self, q: &str, prefer_animation: bool) -> Result<Option<Schedule>, reqwest::Error> {
        let list: Value = self.client.get("https://api.tvmaze.com/search/shows").query(&[("q", q)]).send().await?.error_for_status()?.json().await?;
        // Meilleure fiche : nom qui correspond, en cours de diffusion, animation si c'est un animé, la plus récente
        let best = list
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| &r["show"])
            .filter(|s| s["name"].as_str().is_some_and(|n| similar(q, n)))
            .max_by_key(|s| {
                let status = match s["status"].as_str() {
                    Some("Running") => 30,
                    Some("In Development") | Some("To Be Determined") => 20,
                    _ => 0,
                };
                let anim = if prefer_animation && s["type"].as_str() == Some("Animation") { 15 } else { 0 };
                let exact = if norm(s["name"].as_str().unwrap_or_default()) == norm(q) { 100 } else { 0 };
                let year: i64 = s["premiered"].as_str().and_then(|p| p.get(..4)).and_then(|y| y.parse().ok()).unwrap_or(1900);
                exact + status + anim + (year - 1900) / 10
            });
        let Some(id) = best.and_then(|s| s["id"].as_i64()) else { return Ok(None) };
        let v: Value = self
            .client
            .get(format!("https://api.tvmaze.com/shows/{id}"))
            .query(&[("embed[]", "episodes"), ("embed[]", "nextepisode")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let episodes = v["_embedded"]["episodes"].as_array().into_iter().flatten().filter_map(tvmaze_ep).collect();
        Ok(Some(Schedule {
            show: v["name"].as_str().unwrap_or(q).to_string(),
            status: v["status"].as_str().unwrap_or_default().to_string(),
            source: "tvmaze",
            next: tvmaze_ep(&v["_embedded"]["nextepisode"]),
            episodes,
        }))
    }
}

#[cfg(test)]
mod tests {
    /// `cargo test live_schedule -- --ignored --nocapture` (réseau requis)
    #[test]
    #[ignore]
    fn live_schedule() {
        let s = super::Schedules::new();
        tauri::async_runtime::block_on(async {
            for (t, k) in [(vec!["JoJos Bizarre Adventure".to_string(), "JoJo's Bizarre Adventure".to_string()], "series"), (vec!["Dreamland".into()], "anime"), (vec!["Loki".into()], "series")] {
                let t = &t;
                match s.get(t, k).await {
                    Some(x) => println!(
                        "{t:?} -> {} [{} via {}] {} épisodes, prochain: {:?}",
                        x.show,
                        x.status,
                        x.source,
                        x.episodes.len(),
                        x.next.map(|e| (e.season, e.number, e.at, e.airstamp))
                    ),
                    None => println!("{t:?} -> rien"),
                }
            }
        });
    }
}
