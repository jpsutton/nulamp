# 07 — Playback Engine

This document defines playback state, transport commands, playlist advance, shuffle, the audio processing chain, ReplayGain, the output stage and network streaming.

---

## 1. State

| State | Meaning |
|---|---|
| Stopped | Nothing open. Position display is cleared |
| Playing | Decoding and output running |
| Paused | Open, output paused. Time display blinks |

Remote-control "is playing" value: 0 stopped, 1 playing, 3 paused.

Other state:

- **Current playlist position** (index). Exists even when stopped.
- **Shuffle** on/off (default off). **Repeat** on/off (default off). **Manual advance** on/off (default off).
- **Stop after current** (one-shot flag, not saved; cleared whenever a track starts).
- Volume 0..255 (default 200). Balance −127..127 (default 0).

The length of the current item can be unknown (live streams). Seeking is allowed only when the decoder says the item is seekable.

---

## 2. Transport commands

Shift and Ctrl modify the transport buttons, their keys and their menu items.

| Command | Plain | Shift | Ctrl |
|---|---|---|---|
| Previous (Z) | Previous track (§2.1) | Rewind 5 s | Go to start of list |
| Play (X) | §2.2 | Open file(s) | Open URL |
| Pause (C) | Toggle pause, only while playing | — | — |
| Stop (V) | Stop | Stop with fade-out (§2.3) | Toggle "stop after current" (only while playing) |
| Next (B) | Next track (§2.4) | Fast-forward 5 s | Go to end of list |

### 2.1 Previous

1. If a queue extension provides a "previous" item (§3.4), play it.
2. With shuffle on: step back one place in the shuffle order. At the start of the shuffle order, select the first slot and do not restart playback.
3. With shuffle off: position − 1. At the top of the list: wrap to the last entry if repeat is on, else do nothing.
4. If playing, start the new track. If stopped, only move the position.

The reference player has **no** "restart the current track if more than N seconds in" rule. Previous always changes track. An implementation may offer that rule as a non-default option.

### 2.2 Play

- Stopped: play the current entry. If the playlist is empty, open the file dialog.
- Paused: unpause.
- Playing: **restart** the current track from the beginning.

### 2.3 Stop with fade-out

Ramp the volume linearly from its current value to 0 over about 2 seconds (256 steps of 8 ms), wait 100 ms, stop, then restore the volume setting. It must not block the user interface.

### 2.4 Next

1. If a queue extension provides a "next" item, play it.
2. Shuffle on: step forward in the shuffle order. If the end of the order is reached: with repeat off, do nothing; with repeat on, reshuffle (§3.2) and continue at the first slot. Never play the same entry twice in a row when the list has more than one entry.
3. Shuffle off: position + 1. At the end: wrap to the first entry if repeat is on, else do nothing.
4. If the list was cleared while a track was playing, start at entry 1.
5. If playing, start the new track. If stopped, only move the position.

### 2.5 Other navigation

- Jump 10 tracks back/forward (Numpad 1/3): equal to 10 Previous/Next commands.
- Seek ±5 s (Left/Right): relative to the current output position, clamped at 0. Only when seekable. If the seek fails, treat it as end of track.
- Jump to time (Ctrl+J): a dialog shows the length and current position as `M:SS`. Input `SS` (seconds) or `M:SS`. Only when seekable.
- Jump to file (J): 10 §6.

### 2.6 Volume and balance keys

- Up/Down (and Numpad 8/2): volume ±4 (of 255).
- Mouse wheel: ±8 per notch (02 §12.5).
- Menu "volume up/down": ±20.
- Balance keys exist only in the equalizer window (03 §1.7).

---

## 3. End of track and play order

### 3.1 End of track

When the output has played all audio of the current track:

1. If a queue extension provides a next item, play it (this bypasses shuffle, repeat, manual advance and stop-after-current).
2. Else, if the playlist is empty or "stop after current" is set: clear "stop after current" and stop. Stay on the same entry.
3. Else, if **manual advance** is on: if repeat is on, play the same track again ("repeat track"); else stop and stay on the same entry.
4. Else, if the current entry has per-entry repeats left (a playlist `Repeat` count, 10 §2), play it again.
5. Else, with shuffle off: move to position + 1. If past the end: with repeat on, go to entry 1 and play; with repeat off, stop (stay on the last entry).
6. Else, with shuffle on: step forward in the shuffle order. If the order is exhausted and repeat is off, stop. If the new entry equals the old one and the list has more than one entry, step again. Play.

**Repeat modes, as shown to the user:**

| Repeat | Manual advance | Result | Marquee text |
|---|---|---|---|
| Off | Off | Play the list once | `Repeat: Off` |
| On | Off | Loop the list | `Repeat: Playlist` |
| On | On | Loop the current track | `Repeat: Track` |
| Off | On | Stop after each track | `Manual Advance: On` |

### 3.2 Shuffle algorithm

- **Random source:** a high-quality generator (the reference uses a Mersenne Twister) seeded from system entropy at start-up. Shuffle order is not reproducible across sessions.
- **Shuffle order:** a permutation of all playlist indices plus a cursor into it.
- **Build:** whenever the list length changed or the list was modified, rebuild the permutation with a uniform shuffle (Fisher–Yates) and set the cursor to 0. **On the first build after start-up**, put the current entry in slot 0 and shuffle only the rest, so the player resumes on the same song.
- **Step:** move the cursor by +1 or −1. Play the entry in that slot.
- **End of order with repeat on — "morph":** set the cursor to 0 and partially reshuffle so that recently played songs do not repeat soon:
  - window = max(list length × morph rate / 100, 2), where morph rate is a preference 0..50 (default 50).
  - For each position x from 0 to length − 2, shuffle the slots from x to min(x + window, length) − 1.
  - Rate 50 means about half of the other songs play before any song repeats. Rate 0 means the order changes very little between passes.
  - Skip the morph for lists of 2 or fewer entries.
- **End of order with repeat off:** the cursor stays at the last slot; report "end reached".
- **Before the start:** the cursor stays at slot 0; report "start reached".
- **Adding an entry:** insert its index at a uniformly random slot in the order (it may land before the cursor and not play this pass). Shift indices that moved.
- **Removing an entry:** remove its slot, renumber higher indices, and move the cursor back by one if the removed slot was at or before it.
- **Turning shuffle on while stopped:** rebuild the order and select slot 0 (a random entry).
- **Start of list / end of list with shuffle on:** go to slot 0 / the last slot.
- Sorting or reversing the list may keep or rebuild the order. Rebuilding is recommended.

### 3.3 Unplayable entries

- If an entry cannot be opened (no decoder, file missing, decoder error), skip to the next entry as if the track had ended.
- Count consecutive failures. If every entry in the list fails in a row, stop. Do not loop forever.
- If no audio output device is available, stop and show an error.
- Asynchronous decoder failures (for example a stream that cannot connect) also count as end of track.

### 3.4 Queue (extension)

The reference core has no play queue (a popular third-party extension provided one). It is recommended:

- A first-in-first-out list of playlist entries.
- **Q** in the playlist (and in Jump to file) adds or removes the selected entries.
- Queued rows show their queue position, for example `[1]`, before the title.
- On Next and on end of track, if the queue is not empty, play and remove its head. This bypasses shuffle, repeat and manual advance.
- Adjust stored indices when entries are inserted or removed.

---

## 4. Audio processing chain

Process in this order, per block of audio:

| Stage | Notes |
|---|---|
| 1. Decode | Native sample format, any channel count and sample rate |
| 2. Gain | One combined gain: EQ preamp (03 §3.2, only when EQ is active) × ReplayGain or "no tag" gain (§5) × ReplayGain preamp |
| 3. Equalizer | 10 bands and limiter (03 §4). Bypassed when off or flat |
| 4. DSP effect plugin | Optional. May change the sample count (time-stretch, pitch) |
| 5. Visualization tap | Classic visualizer and external plugins read here (06 §2) |
| 6. Output stage | Resampling (if needed), crossfade/fades, volume and balance, device |

Requirements:

- Process in 32-bit float internally. Convert to the device format at the end. Use dither (TPDF) when reducing bit depth, controlled by a preference (default on).
- Support at least 8-, 16-, 24- and 32-bit integer and 32-bit float input; mono, stereo and multichannel (up to 8 channels); sample rates 8–384 kHz.
- Preferences (reference defaults): output bit depth 16 (24 allowed when enabled); allow surround / multichannel (on); force mono (off); dither (on).
- The decoder of a format may apply its own EQ or ReplayGain (for example a decoder that equalizes in the frequency domain). Then it must say so, and the core must skip that stage. A new implementation should do both stages centrally only.
- Volume and balance are applied at the **output** stage, after the visualization tap.

---

## 5. ReplayGain

### 5.1 Settings

| Setting | Default | Values |
|---|---|---|
| Enable | Off | |
| Source | Track | Track / Album |
| Mode | Apply gain, prevent clipping | Apply gain / Apply gain, prevent clipping / Normalize to peak / Prevent clipping only |
| Preferred source only | Off | When on, do not fall back to the other source |
| Gain for files without ReplayGain | −6.0 dB | −14 .. +6 dB, 0.1 dB steps |
| Preamp for files with ReplayGain | 0.0 dB | −14 .. +6 dB, 0.1 dB steps |

### 5.2 Gain computation (per track)

1. Read the preferred source's gain (dB) and peak (linear). If the gain is missing and "preferred source only" is off, use the other source.
2. If no gain was found: gain factor = 10^(no-tag dB / 20). No preamp. Done.
3. Parse the gain as a decimal number (a leading `+` and a trailing ` dB` are allowed; always use `.` as decimal separator). Parse the peak; if missing, peak = 1.0.
4. Mode:
   - Apply gain: factor = 10^(dB / 20).
   - Apply gain, prevent clipping: factor = min(10^(dB / 20), 1 / peak).
   - Normalize: factor = 1 / peak.
   - Prevent clipping only: factor = 1 / peak if peak > 1, else 1.
5. Multiply by the preamp factor 10^(preamp dB / 20). (The reference applies the preamp after the clipping check.)

### 5.3 Tag sources

| Container | Keys |
|---|---|
| ID3v2 | `TXXX` frames with descriptions `replaygain_track_gain`, `replaygain_track_peak`, `replaygain_album_gain`, `replaygain_album_peak` (case-insensitive match recommended) |
| APEv2 | Same key names, case-insensitive |
| Vorbis comment (FLAC, Ogg, Opus) | `REPLAYGAIN_TRACK_GAIN`, etc. For Opus also honour the header output gain and `R128_TRACK_GAIN` / `R128_ALBUM_GAIN` (Q7.8 dB relative to −23 LUFS; add 5 dB to convert to the ReplayGain reference) |
| MP4 | Freeform atoms `----:com.apple.iTunes:replaygain_track_gain` etc. (accept any mean) |
| MP3 LAME header | Track gain from the LAME info tag when no tag exists (optional) |
| WMA / ASF | Attributes with the same lower-case names |

### 5.4 Scanner (optional)

A ReplayGain scanner computes and writes these tags. Use the ReplayGain 1.0 algorithm (reference level 89 dB SPL) for compatibility, or EBU R128 loudness with the conversion gain = −18 LUFS − measured loudness. Album gain covers all tracks of an album. Options: ask before writing; treat each album separately; skip albums that already have tags.

---

## 6. Output stage

### 6.1 Buffering

| Setting | Default |
|---|---|
| Buffer length | 2000 ms (range 200–20000) |
| Prebuffer on start and after seek | 500 ms |
| Volume change smoothing | 150 ms ramp |

- The output reports the **heard position** (written audio minus device latency minus resampler latency). This drives the time display, seek bar and visualization.
- Seek: discard buffered audio and restart at the new position.

### 6.2 Fades

| Fade | Default | Length |
|---|---|---|
| On pause and stop | **On** | 333 ms |
| On seek | **On** | 333 ms (fade out old position, fade in new) |
| On start of playback | Off | 333 ms |
| On first start (application launch) | Off | 333 ms |
| At end of each track (crossfade) | Off | 333 ms (user-adjustable; typical 2000–10000 ms) |

- Pause with fade: fade out, then pause. Unpause resumes without a jump.
- Stop with fade: the old audio fades out in the background; the user interface is not blocked.
- Fades follow a linear gain curve. An equal-power curve may be offered for crossfade.

### 6.3 Gapless playback

- When the remaining buffered audio of the current track drops below a threshold (500 ms), start decoding the next track.
- If the next track has the same sample format, continue writing to the same output stream with **no gap and no overlap** (sample-accurate).
- If the format differs, let the old stream drain, then open a new stream (or resample to the current format; recommended).
- Decoders must trim encoder delay and padding (09 §2) so album tracks join without clicks.

### 6.4 Crossfade

- When crossfade is on with length F: start the next track when F of audio remains in the current track. Fade the old track out and the new track in over F, mixing them.
- Do not crossfade on manual track changes unless a preference says so (recommended option, default off).
- Do not crossfade tracks of the same album when a preference "gapless for albums" is on (recommended, default on).

### 6.5 Silence removal (optional)

Option to skip leading and trailing silence below a threshold (default −40 dB). Default off.

### 6.6 Volume and balance mapping

- Volume factor = volume / 255 (linear amplitude). This matches the reference default. An optional logarithmic curve may be offered.
- Balance: attenuate the opposite channel linearly. With balance b in −127..127: if b > 0, left gain = 1 − b / 128; if b < 0, right gain = 1 + b / 128. Centre: no change.
- For multichannel audio, apply balance to front left/right pairs only.

### 6.7 Devices and back ends

- One output abstraction with back ends per platform (for example WASAPI / CoreAudio / PipeWire / PulseAudio / ALSA), or a portable library.
- Preferences: device selection (default = system default), exclusive mode where available, buffer length, prebuffer, fades, crossfade, gapless.
- Handle device loss (unplugged headphones): fall back to the default device without stopping, when possible.

### 6.8 Write to file ("disk writer", optional)

An output that writes decoded audio to files instead of a device:

- One file per track, or all tracks into one file.
- Output folder, or "same folder as source".
- File name pattern using title formatting (10 §5), with an optional playlist-number prefix.
- Format: WAV (or any encoder from 09 §4).
- Runs as fast as decoding allows.

---

## 7. Decoder interface (behavioural)

A decoder module provides:

- **Claim:** which file extensions and URL schemes it handles, and an optional content check ("is this my file?"), consulted before extension matching.
- **Open/Play:** start decoding. Return quickly. Report sample rate, channels, bits, bit rate (kbps), seekable or not, length (or unknown).
- **Pause / unpause / stop**.
- **Seek** to a time in milliseconds.
- **Info updates:** bit rate changes (VBR), channel count, "in sync" state (02 §4), title changes (streams).
- **End of stream:** signal when all audio is decoded. The core signals "track ended" only when the output has played it.
- **Metadata read and write** by field name (10 §7).
- **Decode to PCM without playing** (for format conversion, ripping and the ReplayGain scanner).

**Decoder selection for a location:**

1. The first decoder whose content check accepts it.
2. Else, the first decoder whose extension list contains the extension (case-insensitive). For URLs, ignore the query string (`?...`) when finding the extension.
3. Else, the decoder for the "assumed extension of unknown files" preference (default `mp3`). This makes extension-less stream URLs play.

Better: sniff the content (magic bytes, HTTP content type) when the extension does not match any decoder.

---

## 8. Network streaming

### 8.1 Supported locations

- `http://`, `https://`.
- `icy://`, `sc://`, `shoutcast://`, `uvox://`: treat as HTTP to the same host and port.
- `file://` URIs: treat as local files (percent-decode).
- `cda://` and others: see 09.

### 8.2 Playlist URLs

- If a URL's extension (ignoring the query string) is a playlist type (`.pls`, `.m3u`, `.m3u8`, `.asx`, `.xspf`, `.b4s`, `.wpl`), download it and load its entries. Also detect playlists by HTTP content type (`audio/x-scpls`, `audio/x-mpegurl`, `audio/mpegurl`, `application/vnd.apple.mpegurl`, `application/xspf+xml`, `video/x-ms-asf`).
- HLS (`#EXT-X-TARGETDURATION` in an M3U8): play as one stream, not as a playlist (recommended extension).
- Follow redirects (at most 10).

### 8.3 Stream request

- HTTP GET with `Icy-MetaData: 1`, `Accept: */*`.
- The user agent should identify the new player. Some servers need a SHOUTcast-compatible user agent; offer a preference for a custom user agent string.
- Accept `ICY 200 OK` as a valid status line (old SHOUTcast servers), as well as any `HTTP/1.x 2xx`.
- Use HTTP Range requests to seek in streams that report `Content-Length` and `Accept-Ranges`.
- Basic authentication: prompt for user name and password on 401, remember per realm.
- Proxy: HTTP proxy `[user:pass@]host:port`, optional "only for port 80", and the system proxy settings.
- Connect timeout 15 s.

### 8.4 Headers

| Header | Use |
|---|---|
| `icy-name` | Station name; initial title |
| `icy-genre` | Stream genre (metadata field `streamgenre`) |
| `icy-url` | Station home page (`streamurl`) |
| `icy-metaint` | Interval of in-band metadata, in bytes |
| `icy-br` | Bit rate (informational; the decoded bit rate wins) |
| `icy-notice2` | Server description (text before `<BR>`) |
| `Content-Type` | Codec selection (`audio/mpeg`, `audio/aac`, `audio/aacp`, `audio/ogg`, `audio/flac`, …) |
| `Content-Disposition` `filename=` | Title for plain HTTP files |

Before headers arrive, the title is the last path part of the URL (or the whole URL).

### 8.5 In-band metadata

- After every `icy-metaint` bytes of audio, read 1 length byte L, then L × 16 bytes of metadata. L = 0 means no change.
- Metadata is a sequence of `Key='Value';` pairs. Find each value between `='` and the next `';`.
- `StreamTitle`: the current song (metadata field `streamtitle`). Decode as UTF-8 if valid, else Windows-1252.
- `StreamUrl`: optional link.
- **Displayed title:** `StreamTitle (Station Name)` when the "show station name in title" preference is on (default on), else `StreamTitle`. If StreamTitle is empty: `(Station Name)`.
- On each change, update the playlist entry title, the marquee and the window caption.
- Also support Ogg/Vorbis/Opus stream comments (title changes at chained-stream boundaries) and SHOUTcast 2 / Ultravox framing (optional).

### 8.6 Buffering

| Setting | Default |
|---|---|
| Network buffer | 64 KB (minimum 32 KB) |
| Prebuffer before playback | 40% of the buffer, scaled by bit rate: needed % = min(100, setting × clamp(kbps, 8, 320) / 128). At start the target is at least half the setting |
| Rebuffer after underrun or seek | 10% |

- While buffering, set the title to `[Buffer: N%] <title>` (N = percent of the target reached), clear the "in sync" light, and optionally draw a progress bar in the visualizer.
- Show `[Connecting] <url>` while connecting and short status texts for errors (`[Access denied]`, `[Server full]`, `[Timed out]`).
- Length is unknown for live streams: time display shows elapsed only; remaining mode shows elapsed.

### 8.7 Reconnection

The reference player does not reconnect: a dropped stream ends the track and the playlist advances normally. An implementation should offer automatic reconnection (recommended, default on: up to 5 tries with increasing delay).

### 8.8 Saving streams (optional)

Option to save the raw stream to a folder, named from the station name (append `.mp3` or the correct extension).
