# 09 — Media Format Support

This document lists the formats the player must decode and encode, with the per-format behaviour that users of the original player rely on. Metadata field mapping is in 10 §7.

Priority levels:

- **P1** — Winamp 2.x baseline. Required for a 1.0 release.
- **P2** — Formats added by later Winamp versions. Required for "all major features".
- **P3** — Optional.

---

## 1. Format matrix

| Format | Extensions / schemes | Priority | Seek | Gapless | Tags read | Tags write | Suggested library |
|---|---|---|---|---|---|---|---|
| MPEG audio layer 1/2/3 | mp3, mp2, mp1 (also mpa) | P1 | Yes | Yes | ID3v1, ID3v2.2/2.3/2.4, APEv2, Lyrics3 v2 (title/artist/album) | ID3v1, ID3v2, APEv2 | mpg123 |
| WAV and other uncompressed/PCM files | wav, aif, aiff, aifc, au, snd, voc, w64, rf64, caf, iff, raw, … | P1 | Yes | Yes | RIFF INFO, ID3 chunk | Optional | libsndfile |
| Ogg Vorbis | ogg, oga | P1 | Yes | Yes | Vorbis comment, embedded pictures | Vorbis comment | libvorbis |
| MIDI and related | mid, midi, rmi, kar (+ P3: hmp, hmi, xmi, mus, cmf, mids, gmd, mss, miz, hmz) | P1 | Yes | n/a | Text/copyright meta events, RIFF INFO (rmi) | No | FluidSynth + SoundFont |
| Tracker modules | mod, s3m, xm, it, mtm, 669, ult, far, okt, ptm, stm, amf, med, mptm, mo3, umx, … (everything the library supports) | P1 | Yes | n/a | Title, artist, tracker, message | No | libopenmpt |
| Audio CD | cda, `cda://` | P1 | Yes | Yes | CD-Text, online lookup | Local cache only | libcdio + cdparanoia |
| HTTP / SHOUTcast / Icecast streams | http, https, icy, sc, shoutcast, uvox | P1 | Only with Content-Length | n/a | ICY, stream comments | — | Built in (07 §8) |
| AAC (ADTS raw), HE-AAC | aac, adts | P2 | Yes (approximate) | n/a | ID3v2, APEv2 | Optional | FFmpeg or fdk-aac |
| MPEG-4 audio | m4a, mp4, m4b, m4r (AAC, HE-AAC, ALAC, MP3-in-MP4) | P2 | Yes | Yes (iTunSMPB, edit lists) | MP4 atoms | MP4 atoms | FFmpeg + tag library |
| FLAC | flac (and FLAC in Ogg: oga) | P2 | Yes | Yes | Vorbis comment, PICTURE blocks | Yes | libFLAC |
| WavPack | wv (+ wvc correction files) | P2 | Yes | Yes | APEv2, ID3v1 | APEv2 | libwavpack |
| Windows Media Audio | wma, asf | P2 | Yes | — | ASF attributes | Optional | FFmpeg (no DRM) |
| Opus | opus | P2 (not in reference; strongly recommended) | Yes | Yes | Vorbis comment, R128 gains | Yes | libopus / opusfile |
| Monkey's Audio, Musepack, TTA, ALAC in CAF | ape, mpc, tta | P3 | Yes | Yes | APEv2 | APEv2 | FFmpeg |
| Line input | `linein://` | P3 | No | n/a | — | — | platform capture |
| Video | avi, mkv, webm, mp4, m4v, mov, flv, nsv, wmv, asf, mpg, mpeg, m2v | P3 | Yes | — | Container tags | No | FFmpeg |

General rules:

- Decoders are selected as in 07 §7. When an extension is claimed by more than one decoder, the user can set the preferred decoder per extension.
- Lists of extensions handled by each decoder are user-editable (add or remove extensions), with a "reset to default" action.
- **Unknown extensions** default to the MPEG audio decoder (setting "assumed extension of unknown files", default `mp3`). Content sniffing is recommended in addition.
- All decoders must provide "decode to PCM" (08 §3.5) and metadata get (08 §3.4).

---

## 2. MPEG audio (MP3/MP2/MP1)

- MPEG-1, MPEG-2 and MPEG-2.5, layers I, II, III. Mono, stereo, joint stereo, dual channel.
- **Gapless:** trim encoder delay and padding. Sources, in order: LAME/Xing info tag; FhG VBRI header; iTunes `iTunSMPB` comment frame in ID3v2 (hex fields: encoder delay, padding, total samples). Also account for the decoder's own delay (529 samples for layer III synthesis) when the encoder information does not already include it.
- **Length and seeking:** for VBR files, use the Xing/LAME table of contents (100 entries) or the VBRI table. If the LAME header's byte count disagrees strongly with the file size, ignore it. Without a header, estimate from the first frames' bit rate. An optional "full scan" for exact length is recommended (default off).
- **Bit rate display:** current frame bit rate by default. Option: show the average bit rate for VBR files.
- **Options to keep:** output quality (full / half / quarter sample rate, i.e. downsampling), reverse stereo, CRC check.
- **Equalizer:** the reference decoder could equalize in the frequency domain. Not required; use the central EQ (03 §4).
- **Tags:** read order ID3v2, then APEv2, then Lyrics3 v2, then ID3v1; the first tag that has a field wins (field by field). Each tag type can be disabled for reading. File layout from the end: `[ID3v2][audio][APEv2][Lyrics3v2][ID3v1]`. Also skip an appended ID3v2.4 tag (footer `3DI`).
- **Tag writing defaults:** update existing ID3v1, ID3v2, APEv2; create ID3v1 and ID3v2 when missing; do not create APEv2 when missing. ID3v2 version written: keep 2.4 for existing 2.4 tags; otherwise 2.3 (option: always 2.4). Text encoding: UTF-16 with BOM for ID3v2.3, UTF-8 for 2.4. Write in place when the new tag fits in the old tag plus padding; otherwise rewrite the file through a temporary file and replace it atomically. Keep unknown frames when rewriting.
- **Read encoding of 8-bit tags** (ID3v1, Latin-1 ID3v2 frames): option "Latin-1" or "system code page". Default: Latin-1, but try UTF-8 first if the bytes are valid UTF-8 (many tools write UTF-8 into ID3v1).
- **Streams:** MP3 and AAC SHOUTcast streams (07 §8). Ultravox (SHOUTcast 2) framing optional.
- **Stream saving:** optional (07 §8.8).

## 3. AAC / MPEG-4

- AAC-LC, HE-AAC v1/v2 (SBR, PS), ALAC, MP3 in MP4.
- Gapless: `iTunSMPB` freeform atom, or MP4 edit lists. Seek with sample accuracy.
- Tags: iTunes `ilst` atoms (10 §7). Cover art `covr`.
- Writing tags must not corrupt the file. Place `moov` before `mdat` when rewriting (fast start).
- Raw ADTS AAC files and AAC streams (`audio/aac`, `audio/aacp`).

## 4. Encoders (for conversion and CD ripping)

| Output | Settings and defaults | Library |
|---|---|---|
| MP3 | CBR / VBR / ABR; CBR 128 kbps default; VBR quality "standard"; joint stereo; up to 320 kbps | LAME |
| FLAC | Compression level 0–8, default 5 | libFLAC |
| Ogg Vorbis | VBR quality 0.4 default (−1..10), or ABR with nominal/min/max kbps | libvorbis |
| Opus (recommended addition) | VBR bit rate, default 128 kbps stereo | libopus |
| AAC / M4A | VBR or CBR; LC / HE / HEv2 | fdk-aac (licence restrictions) or FFmpeg AAC |
| WAV / AIFF | Bit depth, header on/off | built in / libsndfile |

Encoders are modules with: list of output types; create(channels, sample rate, bits) → encoder; encode(PCM) → bytes; finish; settings stored per caller.

**Format converter (P2):** convert selected playlist entries to another format. File-name pattern via title formatting, default `<Artist> - <Album>/## - <Title>`. Copy metadata and cover art to the output.

## 5. Audio CD

- Locations: `cda://<drive>` for the whole disc (expands to one entry per track) and `cda://<drive>,<track>` for one track. Also accept `.cda` stubs on Windows. Drive is a drive letter on Windows and a device path or index elsewhere.
- Playback by **digital audio extraction** only (PCM through the normal chain, so EQ, DSP and visualization work). No analog playback.
- Disc information sources, in order: user edits cached locally; CD-Text; online lookup (MusicBrainz by disc ID; optionally gnudb/CDDB by the classic 32-bit disc ID); fall back to "Track N".
- Edits to CD track info are stored in a local cache keyed by disc ID, never written to the disc.
- **Ripping (P2):** secure extraction with error correction (paranoia), read offset correction, encoder selection (§4), file-name pattern, create an M3U for the album.
- Main menu items "Play audio CD" per drive; command-line option to play a drive.

## 6. MIDI

- Formats: standard MIDI file types 0, 1, 2; RIFF RMID (may embed DLS instruments); KAR (karaoke lyrics). P3: HMP/HMI, XMI, MUS (DMX), CMF, MIDS, GMD/MSS, and gzip/zip-compressed MIZ/HMZ.
- Synthesis: software synthesizer with a General MIDI SoundFont (user-selectable), rendered to PCM so it flows through EQ, DSP, visualization and output. Option: system MIDI output device instead (then no visualization).
- Options to keep:
  - Sample rate of the synthesizer (default 44100).
  - Reverb and chorus on/off.
  - Custom SoundFont / DLS file.
  - **Looping:** never / when a loop start is detected / always; loop count (default 1) or infinite. Loop markers: a configurable marker event, and the `loopStart` / `loopEnd` text markers used by some game soundtracks.
  - Reset between tracks: GM, GS, XG; option to disable SysEx.
  - Extra silence after the end (ms).
  - Recover incomplete tracks (default on).
- Lyrics window for KAR files and lyric events (P3).
- Metadata: read-only.

## 7. Tracker modules

- All formats the module library supports. Duration from the library.
- Options (map to the library's render parameters):
  - Sample rate (default 44100; offer 48000, 96000).
  - Stereo separation 0–200% (default 100%).
  - Interpolation: none / linear / cubic / windowed sinc (default windowed sinc).
  - Volume ramping (declick) strength.
  - Repeat count (0 = play once; −1 = infinite) and "loop until stopped".
  - Amiga resampler emulation for MOD (on/off).
  - End behaviour: stop / fade out (default 1 s fade) / continue.
  - Output channels: stereo (default), mono, or quad when surround is allowed.
- Info dialog: title, tracker, message/comments, instruments and samples, live pattern/row/speed (P3).

## 8. Video (P3)

- Container and codec support through a general media library.
- Video window (05 §2), full screen, aspect ratio handling, audio/video track selection, subtitles (optional).
- When video support is disabled (preference), video files play audio only.

## 9. Line input (P3)

`linein://` captures the default input device at 44100 Hz, 16-bit stereo, and plays it through the chain (visualization and DSP work). Not seekable, unknown length.

---

## 10. Windows-only or proprietary features of the reference to replace or drop

| Reference feature | Decision |
|---|---|
| WMA/WMV decoding through Windows Media components, including DRM licence handling | Replace with an open decoder. **Drop DRM-protected content** |
| DirectShow-based playback of misc. video formats | Replace with a general media library |
| Flash (SWF) playback | Drop |
| Media Foundation decoders for AAC, H.264, MPEG-4 | Replace with open decoders |
| Proprietary AAC encoder | Replace (§4) |
| Gracenote CD lookup, auto-tagging, playlist generation | Replace with MusicBrainz (CD lookup) and AcoustID/Chromaprint (auto-tagging); drop playlist generation or implement locally |
| ACM codecs (compressed WAV encode/decode) | Use open decoders for WAV codecs (ADPCM, GSM, µ-law, A-law, MP3-in-WAV); drop ACM encoding |
| DirectSound / waveOut / WASAPI outputs | One portable output abstraction (07 §6.7) |
| DirectMusic / DLS MIDI | Software synthesizer (§6) |
| Direct3D / DirectDraw (visualization, video) | OpenGL / Vulkan / Metal through a portable layer |
| CD burning (proprietary SDK) | Drop, or optional with an open burning library |
| Portable device sync, online services, podcast directory, web views | Out of scope |
| Intel performance libraries | Not needed |
