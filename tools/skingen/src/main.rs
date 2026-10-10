//! Writes the built-in skin into a directory (default: `assets/default-skin`).

fn main() -> std::io::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/default-skin".into());
    std::fs::create_dir_all(&dir)?;
    for (name, data) in nulamp_skingen::files() {
        std::fs::write(std::path::Path::new(&dir).join(&name), data)?;
    }
    println!("wrote built-in skin to {dir}");
    Ok(())
}
