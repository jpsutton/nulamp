//! The provider trait with a remote-style provider, and the registry.

mod common;

use std::sync::Arc;

use common::Fixture;
use nulamp_library::{
    BrowseNode, Capabilities, Error, LibraryEvent, LibraryProvider, LocalLibrary, MockProvider,
    NodeKind, Page, Query, Registry, Sort, SortKey, Track, TrackId,
};

fn titles(tracks: &[Track]) -> Vec<String> {
    tracks.iter().map(Track::display_title).collect()
}

#[test]
fn mock_provider_through_the_trait() {
    let provider: Arc<dyn LibraryProvider> = Arc::new(MockProvider::sample());
    let p = provider.as_ref();
    assert_eq!(p.id(), "mock");
    let caps = p.capabilities();
    assert!(caps.contains(Capabilities::REMOTE));
    assert!(!caps.contains(Capabilities::PLAYLISTS));
    assert!(!caps.contains(Capabilities::REMOVE));
    assert!(matches!(p.remove_tracks(&[]), Err(Error::Unsupported)));
    // The default summary lists every track.
    let everything = p
        .search(&Query::all(), Sort::default(), Page::all())
        .unwrap();
    let summary = p.summary(&BrowseNode::root(), &Query::all()).unwrap();
    assert_eq!(summary.count, everything.total);
    assert_eq!(
        summary.length_ms,
        everything.tracks.iter().map(|t| t.length_ms).sum::<u64>()
    );

    let top = p.browse(&BrowseNode::root()).unwrap();
    let artists = top.iter().find(|n| n.label == "Artists").unwrap();
    let names: Vec<String> = p
        .browse(artists)
        .unwrap()
        .into_iter()
        .map(|n| n.label)
        .collect();
    assert_eq!(names, ["Alpha", "Beta"]);
    let beta = p
        .browse(artists)
        .unwrap()
        .into_iter()
        .find(|n| n.label == "Beta")
        .unwrap();
    let albums = p.browse(&beta).unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].kind, NodeKind::Album);
    let page = p
        .tracks(
            &albums[0],
            &Query::all(),
            albums[0].default_sort,
            Page::all(),
        )
        .unwrap();
    assert_eq!(titles(&page.tracks), ["Ray One", "Ray Ten"]);

    let found = p
        .search(
            &Query::parse("year<2002 ray"),
            Sort::desc(SortKey::Track),
            Page::all(),
        )
        .unwrap();
    assert_eq!(titles(&found.tracks), ["Ray Ten", "Ray One"]);
    let paged = p
        .search(&Query::all(), Sort::asc(SortKey::Title), Page::new(1, 2))
        .unwrap();
    assert_eq!(
        (paged.total, titles(&paged.tracks)),
        (5, vec!["Opening".into(), "Ray One".into()])
    );

    let id = found.tracks[0].id.clone();
    let loc = p.resolve_playback(&id).unwrap();
    assert!(loc.location.starts_with("http://mock/"));
    assert_eq!(
        loc.headers,
        [("Authorization".to_string(), "Bearer mock-token".to_string())]
    );
    let (mime, data) = p.artwork(&id).unwrap().unwrap();
    assert_eq!((mime.as_str(), &data[..4]), ("image/png", &b"\x89PNG"[..]));

    let events = p.subscribe();
    p.set_rating(&id, 4).unwrap();
    p.record_play(&id, 200_000).unwrap();
    assert_eq!(p.track(&id).unwrap().unwrap().rating, 4);
    assert_eq!(p.track(&id).unwrap().unwrap().play_count, 1);
    assert_eq!(
        events
            .try_iter()
            .filter(|e| *e == LibraryEvent::TracksChanged)
            .count(),
        2
    );
    let most = top.iter().find(|n| n.label == "Most played").unwrap();
    let played = p
        .tracks(most, &Query::all(), most.default_sort, Page::all())
        .unwrap();
    assert_eq!(titles(&played.tracks), ["Ray Ten"]);

    // Optional writes the mock lacks fall back to the trait defaults.
    assert!(matches!(p.edit_tags(&id, &[]), Err(Error::Unsupported)));
    assert!(matches!(p.playlists(), Err(Error::Unsupported)));
    assert!(matches!(p.create_playlist("x"), Err(Error::Unsupported)));
    assert!(matches!(p.rescan(), Err(Error::Unsupported)));
    assert_eq!(p.track(&TrackId::new("mock", "nope")).unwrap(), None);
}

#[test]
fn registry_routes_by_provider() {
    let fx = Fixture::new();
    let local = Arc::new(fx.scanned());
    let mock = Arc::new(MockProvider::sample());
    let mut reg = Registry::new();
    reg.register(local.clone());
    reg.register(mock.clone());
    reg.register(Arc::new(MockProvider::sample()));
    assert_eq!(
        reg.providers().iter().map(|p| p.id()).collect::<Vec<_>>(),
        ["local", "mock"],
        "same id replaces"
    );

    let remote_id = TrackId::new("mock", "1");
    let loc = reg.resolve_playback(&remote_id).unwrap();
    assert!(!loc.headers.is_empty());

    let local_track = local
        .search(&Query::parse("hunter"), Sort::default(), Page::all())
        .unwrap()
        .tracks
        .remove(0);
    let loc = reg.resolve_playback(&local_track.id).unwrap();
    assert_eq!(loc.location, local_track.location);

    // finish_play applies the play rule.
    assert!(!reg.finish_play(&local_track.id, 100, 1500).unwrap());
    assert!(reg.finish_play(&local_track.id, 800, 1500).unwrap());
    assert_eq!(local.track(&local_track.id).unwrap().unwrap().play_count, 1);

    assert!(matches!(
        reg.resolve_playback(&TrackId::new("jellyfin", "1")),
        Err(Error::NotFound(_))
    ));
    assert!(reg.unregister("mock").is_some());
    assert!(reg.get("mock").is_none());
    assert!(reg.get(LocalLibrary::ID).is_some());
}
