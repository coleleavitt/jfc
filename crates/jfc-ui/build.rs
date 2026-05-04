use std::path::Path;
use syntect::dumps::dump_to_file;
use syntect::parsing::SyntaxSet;

fn main() {
    let syntaxes_dir = Path::new("syntaxes");
    println!("cargo:rerun-if-changed=syntaxes");
    println!("cargo:rerun-if-changed=build.rs");

    if !syntaxes_dir.exists()
        || std::fs::read_dir(syntaxes_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    {
        return;
    }

    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    builder.add_plain_text_syntax();

    if let Err(e) = builder.add_from_folder(syntaxes_dir, true) {
        eprintln!("cargo:warning=syntax folder load error: {}", e);
    }

    let ss = builder.build();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let pack_path = Path::new(&out_dir).join("extra_syntaxes.packdump");

    match dump_to_file(&ss, &pack_path) {
        Ok(_) => {
            println!(
                "cargo:rustc-env=EXTRA_SYNTAXES_PACK={}",
                pack_path.display()
            );
        }
        Err(e) => eprintln!("cargo:warning=Failed to write syntax pack: {}", e),
    }
}
