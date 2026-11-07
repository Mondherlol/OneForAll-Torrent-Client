fn main() {
    // La clé TMDB est intégrée à la compilation (jamais dans le code source publié)
    println!("cargo:rerun-if-env-changed=ONEFORALL_TMDB_KEY");
    tauri_build::build()
}
