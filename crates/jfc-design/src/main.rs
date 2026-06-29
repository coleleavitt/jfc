//! `jfc-design` CLI — the native design layer the skill pack drives.
//!
//!   jfc-design serve [DIR] [--port N]        local preview server (default cwd)
//!   jfc-design bundle --input F --output F   standalone offline HTML (super_inline_html)
//!                       [--allow-no-thumbnail]
//!   jfc-design handoff --project DIR --feature NAME --files A [B ...]
//!   jfc-design ds [DIR]                       index a design system + write _ds_manifest.json
//!   jfc-design new "Title"                    create a design project under .jfc/design/projects
//!   jfc-design list                           list design projects
//!   jfc-design capabilities [--md|--json]     Claude Design → JFC parity matrix

use std::path::PathBuf;

use jfc_design::{capabilities, design_system, handoff, inline, project::ProjectStore};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    match cmd {
        "serve" => cmd_serve(rest),
        "bundle" => cmd_bundle(rest),
        "handoff" => cmd_handoff(rest),
        "ds" => cmd_ds(rest),
        "new" => cmd_new(rest),
        "list" => cmd_list(rest),
        "capabilities" | "caps" => cmd_caps(rest),
        "help" | "-h" | "--help" => {
            print_help();
            0
        }
        other => {
            eprintln!("unknown command: {other}\n");
            print_help();
            2
        }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}
fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
fn positionals(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") {
            // skip flag and its value unless it's a known bare flag
            if args[i] == "--allow-no-thumbnail" || args[i] == "--md" || args[i] == "--json" {
                i += 1;
            } else {
                i += 2;
            }
        } else {
            out.push(args[i].as_str());
            i += 1;
        }
    }
    out
}

fn cmd_serve(args: &[String]) -> i32 {
    let dir = positionals(args)
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let port: u16 = flag(args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(4321);
    let addr = format!("127.0.0.1:{port}");
    match jfc_design::server::serve(&dir, &addr) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("serve error: {e}");
            1
        }
    }
}

fn cmd_bundle(args: &[String]) -> i32 {
    let Some(input) = flag(args, "--input").or_else(|| positionals(args).first().copied()) else {
        eprintln!("bundle: --input <file.html> required");
        return 2;
    };
    let output = flag(args, "--output")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(input).with_extension("standalone.html"));
    let require_thumbnail = !has(args, "--allow-no-thumbnail");
    match inline::bundle(input, &output, require_thumbnail) {
        Ok(report) => {
            println!("{}", report.summary());
            if report.misses.is_empty() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("bundle error: {e}");
            1
        }
    }
}

fn cmd_handoff(args: &[String]) -> i32 {
    let project = flag(args, "--project")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let Some(feature) = flag(args, "--feature") else {
        eprintln!("handoff: --feature <name> required");
        return 2;
    };
    // collect everything after --files
    let files: Vec<String> = args
        .iter()
        .position(|a| a == "--files")
        .map(|i| {
            args[i + 1..]
                .iter()
                .take_while(|a| !a.starts_with("--"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    match handoff::scaffold(&project, feature, &files) {
        Ok(pkg) => {
            println!("Created {}", pkg.dir.display());
            println!("  README: {}", pkg.readme.display());
            if !pkg.copied.is_empty() {
                println!("  copied: {}", pkg.copied.join(", "));
            }
            println!("Fill in the README with precise tokens, screens, and interactions.");
            0
        }
        Err(e) => {
            eprintln!("handoff error: {e}");
            1
        }
    }
}

fn cmd_ds(args: &[String]) -> i32 {
    let dir = positionals(args)
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    match design_system::index_and_write(&dir) {
        Ok(manifest) => {
            println!("{}", manifest.report());
            println!("\nWrote {}", dir.join("_ds_manifest.json").display());
            if manifest.issues.is_empty() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("ds error: {e}");
            1
        }
    }
}

fn cmd_new(args: &[String]) -> i32 {
    let title = positionals(args).first().copied().unwrap_or("Untitled");
    let store = match ProjectStore::default_in(".") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("new error: {e}");
            return 1;
        }
    };
    match store.create(title) {
        Ok(p) => {
            println!("Created project {} ({})", p.meta().title, p.meta().id);
            println!("  root: {}", p.root().display());
            0
        }
        Err(e) => {
            eprintln!("new error: {e}");
            1
        }
    }
}

fn cmd_list(_args: &[String]) -> i32 {
    let store = match ProjectStore::default_in(".") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("list error: {e}");
            return 1;
        }
    };
    match store.list() {
        Ok(projects) => {
            if projects.is_empty() {
                println!("no design projects under {}", store.base().display());
            } else {
                for p in projects {
                    let ds = if p.is_design_system {
                        " [design-system]"
                    } else {
                        ""
                    };
                    println!("{:<28} {}{ds}", p.id, p.title);
                }
            }
            0
        }
        Err(e) => {
            eprintln!("list error: {e}");
            1
        }
    }
}

fn cmd_caps(args: &[String]) -> i32 {
    if has(args, "--json") {
        match serde_json::to_string_pretty(&capabilities::matrix()) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        }
    } else if has(args, "--md") {
        println!("{}", capabilities::markdown_table());
    } else {
        for c in capabilities::matrix() {
            println!("{:<14} {:<55} {}", c.status.glyph(), c.feature, c.surface);
        }
    }
    0
}

fn print_help() {
    println!(
        "jfc-design — design-artifact layer for JFC\n\n\
         USAGE:\n\
         \x20 jfc-design serve [DIR] [--port N]            local preview server (default cwd, :4321)\n\
         \x20 jfc-design bundle --input F [--output F]     standalone offline HTML (super_inline_html)\n\
         \x20                   [--allow-no-thumbnail]\n\
         \x20 jfc-design handoff --project DIR --feature NAME --files A [B ...]\n\
         \x20 jfc-design ds [DIR]                          index a design system, write _ds_manifest.json\n\
         \x20 jfc-design new \"Title\"                        create a design project\n\
         \x20 jfc-design list                              list design projects\n\
         \x20 jfc-design capabilities [--md|--json]        Claude Design -> JFC parity matrix\n"
    );
}
