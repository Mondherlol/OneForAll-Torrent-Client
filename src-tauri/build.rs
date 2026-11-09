fn main() {
    // La clé TMDB est intégrée à la compilation (jamais dans le code source publié)
    println!("cargo:rerun-if-env-changed=ONEFORALL_TMDB_KEY");
    println!("cargo:rerun-if-env-changed=ONEFORALL_IGDB_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=ONEFORALL_IGDB_SECRET");
    println!("cargo:rerun-if-env-changed=ONEFORALL_IGDB_TOKEN");
    tauri_build::build()
}
