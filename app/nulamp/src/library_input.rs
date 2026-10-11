//! Playback of media-library tracks named by `library://<provider>/<id>` (see
//! `nulamp_library::TrackId::location`): the location is resolved through the library's
//! registry each time the track opens, and the server's stream is played with the request
//! headers the provider asks for.

use std::sync::Arc;

use nulamp_decoders::stream::{StreamFactory, StreamOptions};
use nulamp_decoders::{DecodeError, Decoder, DecoderFactory};
use nulamp_library::{Registry, TrackId};

pub struct LibraryInput {
    pub registry: Arc<Registry>,
    pub streams: StreamFactory,
}

impl DecoderFactory for LibraryInput {
    fn name(&self) -> &str {
        "Media library"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn claims(&self, location: &str) -> bool {
        TrackId::from_location(location).is_some()
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        let id = TrackId::from_location(location)
            .ok_or_else(|| DecodeError::Unsupported(location.to_string()))?;
        let target = self.registry.resolve_playback(&id).map_err(|e| match e {
            nulamp_library::Error::NotFound(what) => DecodeError::NotFound(what),
            e => DecodeError::Open(e.to_string()),
        })?;
        if !nulamp_decoders::stream::is_stream_location(&target.location) {
            return Err(DecodeError::Unsupported(location.to_string()));
        }
        // The entry keeps the library's title; the URL is never shown.
        let options = StreamOptions {
            headers: target.headers,
            keep_title: true,
        };
        Ok(Box::new(
            self.streams.open_stream_with(&target.location, &options)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_library_locations() {
        let input = LibraryInput {
            registry: Arc::new(Registry::new()),
            streams: StreamFactory::default(),
        };
        assert!(input.claims("library://jellyfin-u1/abc"));
        assert!(!input.claims("http://host/a.mp3"));
        assert!(!input.claims("/music/a.mp3"));
        match input.open("library://nobody/1") {
            Err(DecodeError::NotFound(_)) => {}
            Err(e) => panic!("unexpected error {e}"),
            Ok(_) => panic!("opened a track of an unknown provider"),
        }
    }
}
