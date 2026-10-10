//! Technical fields (spec 08 §3.4).

use lofty::properties::FileProperties;

/// Names of the technical fields answered by [`TechInfo::get`].
pub const TECH_FIELDS: [&str; 10] = [
    "length",
    "bitrate",
    "samplerate",
    "channels",
    "bitspersample",
    "lossless",
    "type",
    "family",
    "mime",
    "formatinformation",
];

/// Stream properties of a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TechInfo {
    /// Duration in milliseconds.
    pub length_ms: u64,
    /// Audio bit rate in kbps (average for VBR files).
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bits_per_sample: Option<u8>,
    pub lossless: bool,
    /// Format description, for example `MPEG-1 Layer 3` or `FLAC`.
    pub family: String,
    pub mime: String,
    /// Channel mode or codec detail shown after the channel count, for example `Joint Stereo`.
    pub mode: Option<String>,
    /// File size in bytes.
    pub file_size: Option<u64>,
}

impl TechInfo {
    pub(crate) fn new(props: FileProperties, family: impl Into<String>, mime: &str) -> Self {
        Self {
            length_ms: u64::try_from(props.duration().as_millis()).unwrap_or(u64::MAX),
            bitrate: props
                .audio_bitrate()
                .or(props.overall_bitrate())
                .filter(|b| *b > 0),
            sample_rate: props.sample_rate().filter(|r| *r > 0),
            channels: props.channels().filter(|c| *c > 0),
            bits_per_sample: props.bit_depth().filter(|b| *b > 0),
            lossless: false,
            family: family.into(),
            mime: mime.to_string(),
            mode: None,
            file_size: None,
        }
    }

    /// Value of technical field `name` (one of [`TECH_FIELDS`]).
    ///
    /// `lossless` is `1` or `0`; `type` is always `0` (audio).
    pub fn get(&self, name: &str) -> Option<String> {
        match name {
            "length" => Some(self.length_ms.to_string()),
            "bitrate" => self.bitrate.map(|v| v.to_string()),
            "samplerate" => self.sample_rate.map(|v| v.to_string()),
            "channels" => self.channels.map(|v| v.to_string()),
            "bitspersample" => self.bits_per_sample.map(|v| v.to_string()),
            "lossless" => Some(u8::from(self.lossless).to_string()),
            "type" => Some("0".to_string()),
            "family" => (!self.family.is_empty()).then(|| self.family.clone()),
            "mime" => (!self.mime.is_empty()).then(|| self.mime.clone()),
            "formatinformation" => Some(self.format_information()),
            _ => None,
        }
    }

    /// Multi-line description: format, bit rate, sample rate, channels, bits per sample,
    /// length and file size, each on its own `Label: value` line when known.
    pub fn format_information(&self) -> String {
        let mut lines = vec![format!("Format: {}", self.family)];
        if let Some(b) = self.bitrate {
            lines.push(format!("Bit rate: {b} kbps"));
        }
        if let Some(r) = self.sample_rate {
            lines.push(format!("Sample rate: {r} Hz"));
        }
        if let Some(c) = self.channels {
            match &self.mode {
                Some(m) => lines.push(format!("Channels: {c} ({m})")),
                None => lines.push(format!("Channels: {c}")),
            }
        }
        if let Some(b) = self.bits_per_sample {
            lines.push(format!("Bits per sample: {b}"));
        }
        let secs = self.length_ms / 1000;
        lines.push(format!(
            "Length: {}:{:02} ({} ms)",
            secs / 60,
            secs % 60,
            self.length_ms
        ));
        if let Some(s) = self.file_size {
            lines.push(format!("File size: {s} bytes"));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields() {
        let info = TechInfo {
            length_ms: 125_500,
            bitrate: Some(128),
            sample_rate: Some(44100),
            channels: Some(2),
            bits_per_sample: None,
            lossless: false,
            family: "MPEG-1 Layer 3".into(),
            mime: "audio/mpeg".into(),
            mode: Some("Joint Stereo".into()),
            file_size: Some(1000),
        };
        assert_eq!(info.get("length").as_deref(), Some("125500"));
        assert_eq!(info.get("lossless").as_deref(), Some("0"));
        assert_eq!(info.get("type").as_deref(), Some("0"));
        assert_eq!(info.get("bitspersample"), None);
        assert_eq!(
            info.get("formatinformation").unwrap(),
            "Format: MPEG-1 Layer 3\nBit rate: 128 kbps\nSample rate: 44100 Hz\n\
             Channels: 2 (Joint Stereo)\nLength: 2:05 (125500 ms)\nFile size: 1000 bytes"
        );
    }
}
