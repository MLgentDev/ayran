use std::{env, fs, path::Path};

fn walk(root: &Path, relative: &Path, table: &mut String, directories: &mut String) {
    let mut entries: Vec<_> = fs::read_dir(root.join(relative))
        .expect("read built-in Skill directory")
        .map(|entry| entry.expect("read built-in Skill entry"))
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = relative.join(entry.file_name());
        let kind = entry.file_type().expect("read built-in Skill file type");
        if kind.is_dir() {
            let name = path.to_str().expect("UTF-8 Skill path").replace('\\', "/");
            directories.push_str(&format!("{name:?},\n"));
            walk(root, &path, table, directories);
        } else {
            assert!(kind.is_file(), "built-in Skills contain only regular files");
            let name = path.to_str().expect("UTF-8 Skill path").replace('\\', "/");
            let bytes = fs::read(root.join(&path)).expect("read built-in Skill file");
            table.push_str(&format!("({name:?}, &{bytes:?}),\n"));
        }
    }
}

fn main() {
    let root =
        Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../.agents/skills/ayran");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut table = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    let mut directories = String::from("pub const DIRECTORIES: &[&str] = &[\n");
    walk(&root, Path::new(""), &mut table, &mut directories);
    directories.push_str("];\n");
    table.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("builtin_skills.rs"),
        table + &directories,
    )
    .expect("write embedded Skill table");
}
