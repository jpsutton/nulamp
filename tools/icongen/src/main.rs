//! Writes nulamp's packaged icon files under a repository root (default: the current
//! directory).

fn main() -> std::io::Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let files = nulamp_icongen::files();
    for (name, data) in &files {
        let path = root.join(name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, data)?;
    }
    println!("wrote {} icon files under {}", files.len(), root.display());
    Ok(())
}
