//! The external reader for formats the tag library does not support.

use std::path::Path;

use nulamp_meta::{Error, TechInfo};

fn reader(path: &Path) -> Option<(Vec<(String, String)>, TechInfo)> {
    if path.extension()? != "xyz" {
        return None;
    }
    let fields = vec![
        ("title".to_string(), "Song".to_string()),
        ("artist".to_string(), String::new()),
        ("not-a-field".to_string(), "x".to_string()),
    ];
    let tech = TechInfo {
        length_ms: 1500,
        family: "Test Audio".into(),
        ..TechInfo::default()
    };
    Some((fields, tech))
}

#[test]
fn unsupported_formats_use_the_external_reader() {
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("a.xyz");
    let unknown = dir.path().join("a.qqq");
    std::fs::write(&known, b"not audio at all").unwrap();
    std::fs::write(&unknown, b"not audio at all").unwrap();
    assert!(matches!(
        nulamp_meta::read(&known),
        Err(Error::UnsupportedFormat)
    ));

    assert!(nulamp_meta::set_external_reader(reader));
    assert!(!nulamp_meta::set_external_reader(reader));
    let meta = nulamp_meta::read(&known).unwrap();
    assert_eq!(meta.get("title").as_deref(), Some("Song"));
    assert_eq!(meta.get("artist"), None);
    assert_eq!(meta.get("not-a-field"), None);
    assert_eq!(meta.get("length").as_deref(), Some("1500"));
    assert_eq!(meta.get("family").as_deref(), Some("Test Audio"));
    assert!(meta.supported_fields().is_empty());
    assert!(matches!(
        nulamp_meta::write(&known, &[("title".into(), Some("New".into()))]),
        Err(Error::UnsupportedFormat)
    ));
    assert_eq!(std::fs::read(&known).unwrap(), b"not audio at all");
    assert!(matches!(
        nulamp_meta::read(&unknown),
        Err(Error::UnsupportedFormat)
    ));
}
