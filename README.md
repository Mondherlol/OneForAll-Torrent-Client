<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-horizontal-dark.svg">
    <img src="assets/logo-horizontal.svg" alt="OneForAll — Torrent Client" width="560">
  </picture>
</p>

**Une interface moderne pour tes torrents.** OneForAll s'appuie sur qBittorrent (qui tourne en
arrière-plan, invisible) et te donne une vraie bibliothèque : affiches, séries regroupées,
choix des épisodes, lecture directe dans VLC, notifications…

![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011-0067c0) ![Tauri](https://img.shields.io/badge/Tauri-2-24c8db) ![Rust](https://img.shields.io/badge/Rust-🦀-orange)

## Télécharger

👉 **[Dernière version](https://github.com/Mondherlol/OneForAll-Torrent-Client/releases/latest)** — prends le fichier
`OneForAll_x.y.z_x64-setup.exe` et lance-le. L'app se met ensuite à jour toute seule.

Il faut [qBittorrent](https://www.qbittorrent.org/download) installé (OneForAll le lance et le cache tout seul).

## Ce que ça fait

- 🖼️ **Affiches automatiques** (TMDB, TVmaze, AniList, Steam, Wikipédia, iTunes) et type détecté : films, séries, animés, jeux, logiciels, livres, audio
- 📺 **Séries regroupées** : un épisode par ligne, badge S1 · É6, épisodes téléchargés dans l'ordre
- ✅ **Choix des fichiers à l'ajout** : « les 3 premiers », une saison, tout… avec alerte si le disque est trop petit
- 🔎 **Recherche intégrée** : C411 (Torznab) et Nyaa, magnet, glisser-déposer de .torrent
- ▶️ **Lecture directe** dans VLC, double-clic pour ouvrir
- 🎮 **Jeux & logiciels** : détecte s'ils sont déjà installés (désinstaller, supprimer l'installable), formats consoles (NSP, XCI…)
- 🩺 **Onglet Sources** : seeders, leechers, trackers, pairs — et pourquoi un torrent est bloqué
- 🔔 **Notifications Windows** avec l'affiche quand un téléchargement se termine
- 🧹 **Ménage** : stockage par disque et par type, recherche des .torrent inutilisés
- 🌗 Thème clair/sombre, sélection multiple, clic droit partout, réduit dans la zone de notification

## Développer

Pré-requis : [Rust](https://rustup.rs), [Node.js](https://nodejs.org) 20+, les outils C++ de Visual Studio, et qBittorrent.

```bash
npm install
npm run tauri dev      # lance l'app en mode développement
```

Le code : interface en Svelte 5 (`src/`), cœur en Rust (`src-tauri/src/`) :
`qbit.rs` (API qBittorrent), `media.rs` (reconnaissance des noms), `artwork.rs` (affiches),
`search.rs` (C411 / Nyaa), `installed.rs` (programmes installés), `scan.rs` (fichiers .torrent, disques).

## Publier une version

```bash
npm run release                 # 0.2.0 → 0.2.1
npm run release -- minor        # 0.2.0 → 0.3.0
npm run release -- patch --notes "Ce qui change"
```

Le script compile, **signe** la mise à jour, crée le tag, pousse sur GitHub et publie la release
(installateur + `latest.json` que les apps installées consultent pour se mettre à jour).

Il lit les secrets dans `.env.local` (jamais commité) :

```ini
ONEFORALL_TMDB_KEY=ta_cle_tmdb                       # intégrée à l'app compilée
TAURI_SIGNING_PRIVATE_KEY_PATH=C:\Users\toi\.tauri\oneforall.key
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD=…               # si la clé a un mot de passe
# GITHUB_TOKEN=…                                     # sinon le jeton de Git est utilisé
```

⚠️ Garde la clé de signature (`oneforall.key`) en lieu sûr : sans elle, impossible de publier
des mises à jour que les apps installées accepteront.
