//! ID3v1 genre list (spec 10 Appendix A) and ID3v2 `TCON` decoding.

/// Genre index meaning "no genre".
pub const NO_GENRE: u8 = 255;

/// Genre names by index, 0–254 (spec 10 Appendix A).
#[rustfmt::skip]
pub const GENRES: [&str; 255] = [
    "Blues", "Classic Rock", "Country", "Dance", "Disco", "Funk", "Grunge", "Hip-Hop",
    "Jazz", "Metal", "New Age", "Oldies", "Other", "Pop", "R&B", "Rap",
    "Reggae", "Rock", "Techno", "Industrial", "Alternative", "Ska", "Death Metal", "Pranks",
    "Soundtrack", "Euro-Techno", "Ambient", "Trip-Hop", "Vocal", "Jazz+Funk", "Fusion", "Trance",
    "Classical", "Instrumental", "Acid", "House", "Game", "Sound Clip", "Gospel", "Noise",
    "Alt Rock", "Bass", "Soul", "Punk", "Space", "Meditative", "Instrumental Pop", "Instrumental Rock",
    "Ethnic", "Gothic", "Darkwave", "Techno-Industrial", "Electronic", "Pop-Folk", "Eurodance", "Dream",
    "Southern Rock", "Comedy", "Cult", "Gangsta Rap", "Top 40", "Christian Rap", "Pop/Funk", "Jungle",
    "Native American", "Cabaret", "New Wave", "Psychedelic", "Rave", "Showtunes", "Trailer", "Lo-Fi",
    "Tribal", "Acid Punk", "Acid Jazz", "Polka", "Retro", "Musical", "Rock & Roll", "Hard Rock",
    "Folk", "Folk-Rock", "National Folk", "Swing", "Fast-Fusion", "Bebop", "Latin", "Revival",
    "Celtic", "Bluegrass", "Avantgarde", "Gothic Rock", "Progressive Rock", "Psychedelic Rock", "Symphonic Rock", "Slow Rock",
    "Big Band", "Chorus", "Easy Listening", "Acoustic", "Humour", "Speech", "Chanson", "Opera",
    "Chamber Music", "Sonata", "Symphony", "Booty Bass", "Primus", "Porn Groove", "Satire", "Slow Jam",
    "Club", "Tango", "Samba", "Folklore", "Ballad", "Power Ballad", "Rhythmic Soul", "Freestyle",
    "Duet", "Punk Rock", "Drum Solo", "A Cappella", "Euro-House", "Dance Hall", "Goa", "Drum & Bass",
    "Club-House", "Hardcore", "Terror", "Indie", "BritPop", "Afro-Punk", "Polsk Punk", "Beat",
    "Christian Gangsta Rap", "Heavy Metal", "Black Metal", "Crossover", "Contemporary Christian", "Christian Rock", "Merengue", "Salsa",
    "Thrash Metal", "Anime", "JPop", "Synthpop", "Abstract", "Art Rock", "Baroque", "Bhangra",
    "Big Beat", "Breakbeat", "Chillout", "Downtempo", "Dub", "EBM", "Eclectic", "Electro",
    "Electroclash", "Emo", "Experimental", "Garage", "Global", "IDM", "Illbient", "Industro-Goth",
    "Jam Band", "Krautrock", "Leftfield", "Lounge", "Math Rock", "New Romantic", "Nu-Breakz", "Post-Punk",
    "Post-Rock", "Psytrance", "Shoegaze", "Space Rock", "Trop Rock", "World Music", "Neoclassical", "Audiobook",
    "Audio Theatre", "Neue Deutsche Welle", "Podcast", "Indie Rock", "G-Funk", "Dubstep", "Garage Rock", "Psybient",
    "Glam Rock", "Dream Pop", "Merseybeat", "K-Pop", "Chiptune", "Grime", "Grindcore", "Indietronic",
    "Indietronica", "Jazz Rock", "Jazz Fusion", "Post-Punk Revival", "Electronica", "Psychill", "Ethnotronic", "Americana",
    "Ambient Dub", "Digital Dub", "Chillwave", "Stoner Rock", "Slowcore", "Softcore", "Flamenco", "Hi-NRG",
    "Ethereal", "Drone", "Doom Metal", "Doom Jazz", "Mainstream", "Glitch", "Balearic", "Modern Classical",
    "Mod", "Contemporary Classical", "Psybreaks", "Psystep", "Psydub", "Chillstep", "Berlin School", "Future Jazz",
    "Djent", "Musique Concrète", "Electroacoustic", "Folktronica", "Texas Country", "Red Dirt", "Arabic", "Asian",
    "Bachata", "Bollywood", "Cajun", "Calypso", "Creole", "Darkstep", "Jewish", "Reggaeton",
    "Smooth Jazz", "Soca", "Spiritual", "Turntablism", "Zouk", "Neofolk", "Nu Jazz",
];

/// Name of genre `index`, or `None` for 255.
pub fn genre_name(index: u8) -> Option<&'static str> {
    GENRES.get(usize::from(index)).copied()
}

/// Index of genre `name` (case-insensitive exact match), or [`NO_GENRE`] when unknown.
pub fn genre_index(name: &str) -> u8 {
    let name = name.to_lowercase();
    GENRES
        .iter()
        .position(|g| g.to_lowercase() == name)
        .map_or(NO_GENRE, |i| i as u8)
}

/// Decodes an ID3v2 `TCON` value to genre text.
///
/// Accepts `(17)`, `17`, `(17)Rock`, `Rock`, `(RX)` (Remix) and `(CR)` (Cover). Text after the
/// references refines them and wins; `((` escapes a literal `(`. Several references, or
/// several ID3v2.4 values (NUL-separated), are joined with `; `. Numbers outside 0–254 stay
/// as written.
pub fn decode_tcon(value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for part in value.split('\0') {
        let genre = decode_one(part.trim());
        if !genre.is_empty() && !out.contains(&genre) {
            out.push(genre);
        }
    }
    out.join("; ")
}

fn decode_one(s: &str) -> String {
    if let Some(name) = numeric_genre(s) {
        return name.to_string();
    }
    let mut refs: Vec<&str> = Vec::new();
    let mut rest = s;
    while rest.starts_with('(') && !rest.starts_with("((") {
        let Some(end) = rest.find(')') else { break };
        let name = match &rest[1..end] {
            "RX" => "Remix",
            "CR" => "Cover",
            inner => match numeric_genre(inner) {
                Some(name) => name,
                None => break,
            },
        };
        if !refs.contains(&name) {
            refs.push(name);
        }
        rest = &rest[end + 1..];
    }
    let text = if rest.starts_with("((") {
        &rest[1..]
    } else {
        rest
    };
    if text.trim().is_empty() {
        refs.join("; ")
    } else {
        text.trim().to_string()
    }
}

/// Genre name for an all-digit string in 0–254.
fn numeric_genre(s: &str) -> Option<&'static str> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u8>().ok().and_then(genre_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_matches_appendix() {
        assert_eq!(genre_name(0), Some("Blues"));
        assert_eq!(genre_name(17), Some("Rock"));
        assert_eq!(genre_name(79), Some("Hard Rock"));
        assert_eq!(genre_name(80), Some("Folk"));
        assert_eq!(genre_name(191), Some("Psybient"));
        assert_eq!(genre_name(233), Some("Musique Concrète"));
        assert_eq!(genre_name(254), Some("Nu Jazz"));
        assert_eq!(genre_name(255), None);
    }

    #[test]
    fn index_lookup() {
        assert_eq!(genre_index("Rock"), 17);
        assert_eq!(genre_index("rOCK"), 17);
        assert_eq!(genre_index("MUSIQUE CONCRÈTE"), 233);
        assert_eq!(genre_index("Psychobilly"), NO_GENRE);
        assert_eq!(genre_index("Rock "), NO_GENRE);
        assert_eq!(genre_index(""), NO_GENRE);
        for i in 0..=254u8 {
            assert_eq!(genre_index(genre_name(i).unwrap()), i);
        }
    }

    #[test]
    fn tcon_forms() {
        for v in ["(17)", "17", "(17)Rock", "Rock"] {
            assert_eq!(decode_tcon(v), "Rock", "{v}");
        }
        assert_eq!(decode_tcon("(RX)"), "Remix");
        assert_eq!(decode_tcon("(CR)"), "Cover");
        assert_eq!(decode_tcon("(4)Eurodisco"), "Eurodisco");
        assert_eq!(decode_tcon("(17)(18)"), "Rock; Techno");
        assert_eq!(decode_tcon("((Weird)"), "(Weird)");
        assert_eq!(decode_tcon("Rock\0Pop"), "Rock; Pop");
        assert_eq!(decode_tcon("300"), "300");
        assert_eq!(decode_tcon("(300)"), "(300)");
        assert_eq!(decode_tcon(""), "");
    }
}
