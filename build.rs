//! Le code recompilé (`src/recomp/code/`) est généré à partir de la cartouche et n'est pas
//! dans le dépôt. Sans lui, un module vide est écrit pour que le projet compile : on
//! obtient alors les outils, qui servent à le générer (voir README.md).

fn main() {
    let dir = std::path::Path::new("src/recomp/code");
    let module = dir.join("mod.rs");
    if !module.exists() {
        std::fs::create_dir_all(dir).expect("création de src/recomp/code");
        let vide = "// Module vide écrit par build.rs : lancer `ladysia-decomp <rom.gba> recomp` pour le générer.\n\
                    pub const CODE_HASH: u64 = 0;\n\
                    pub const DATA_HASH: u64 = 0;\n\
                    pub static COPIES: [(u32, u32, u32, u64); 0] = [];\n\
                    pub static BLOCKS: [(u32, bool, super::Block); 0] = [];\n";
        std::fs::write(&module, vide).expect("écriture de src/recomp/code/mod.rs");
    }
    // version finale : chemin absolu des données à intégrer (un chemin relatif serait
    // cherché à partir de src/)
    println!("cargo:rerun-if-env-changed=LADYSIA_DONNEES");
    if std::env::var_os("CARGO_FEATURE_EMBARQUE").is_some() {
        let p = std::env::var("LADYSIA_DONNEES").expect("LADYSIA_DONNEES=<fichier écrit par la commande donnees> est requis avec --features embarque");
        let p = std::env::current_dir().expect("dossier courant").join(p);
        assert!(p.exists(), "données introuvables : {}", p.display());
        println!("cargo:rerun-if-changed={}", p.display());
        println!("cargo:rustc-env=LADYSIA_DONNEES_ABS={}", p.display());
    }
    println!("cargo:rerun-if-changed=src/recomp/code/mod.rs");
    println!("cargo:rerun-if-changed=build.rs");
}
