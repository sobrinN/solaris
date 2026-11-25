use std::fs;
use std::path::Path;

fn emit_rerun_recursive(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut children: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for child in children {
        if child.is_dir() {
            emit_rerun_recursive(&child);
        } else {
            println!("cargo:rerun-if-changed={}", child.display());
        }
    }
}

fn main() {
    let skills_dir = Path::new("../../skills");
    if skills_dir.exists() {
        emit_rerun_recursive(skills_dir);
    } else {
        println!("cargo:rerun-if-changed=../../skills");
    }
}
