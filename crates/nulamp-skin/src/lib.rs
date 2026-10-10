//! Classic skin loading: containers, BMP decoding, text resources, cursors (spec 01).

pub mod bitmap;
pub mod bmp;
pub mod builtin;
pub mod container;
pub mod cursor;
pub mod fonts;
pub mod ini;
pub mod pledit;
pub mod region;
pub mod skin;
pub mod viscolor;

pub use bitmap::Bitmap;
pub use container::{ContainerError, ContainerKind, SkinEntry, SkinFiles, list_skins};
pub use skin::{CursorRole, CursorShape, Skin, SkinBitmap};
