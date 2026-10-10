//! nulamp: a cross-platform player for classic skins.

mod fake;

use nulamp_skin::{Skin, SkinFiles, builtin};
use nulamp_ui::{Shell, UiPrefs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let skin = match std::env::args().nth(1) {
        Some(path) => match SkinFiles::open(std::path::Path::new(&path)) {
            Ok(files) => Skin::load(&files, &builtin::files()),
            Err(e) => {
                log::error!("cannot load skin {path}: {e}; using the built-in skin");
                builtin::skin()
            }
        },
        None => builtin::skin(),
    };
    Shell::new(fake::FakePlayer::new(), UiPrefs::default(), skin).run()
}
