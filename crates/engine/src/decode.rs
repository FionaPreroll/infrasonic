// SPDX-License-Identifier: GPL-3.0-only
//! Demuxing and decoding with Symphonia, plus a pure Rust Opus decoder.

use opus_decoder::OpusDecoder;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, RawValue, StandardTag, Tag};
use symphonia::core::units::{Time, TimeBase};

use crate::Error;
use crate::gain::ReplayGain;
use crate::mp4edit;

/// Opus always decodes at 48 kHz.
const OPUS_RATE: u32 = 48_000;
/// Largest Opus frame: 120 ms at 48 kHz.
const OPUS_MAX_FRAME: usize = 5760;

enum Codec {
    Symphonia(Box<dyn AudioDecoder>),
    Opus(Box<OpusDecoder>),
}

/// Decodes one audio track to interleaved `f32` samples.
pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    codec: Codec,
    track_id: u32,
    time_base: Option<TimeBase>,
    rate: u32,
    channels: usize,
    duration: Option<f64>,
    buf: Vec<f32>,
    /// Frames to drop after an accurate seek.
    skip_frames: u64,
    /// MP4 edit list, in frames of the codec timeline: audible range `[start, end)`.
    edit: Option<(u64, Option<u64>)>,
    /// Position of the next decoded frame in the codec timeline (only tracked with an edit).
    frame_pos: u64,
    /// First frame to output (the edit start, or the target of a seek).
    window_start: u64,
    replay_gain: Option<ReplayGain>,
    eof: bool,
}

impl std::fmt::Debug for TrackDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackDecoder")
            .field("track_id", &self.track_id)
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .field("duration", &self.duration)
            .finish_non_exhaustive()
    }
}

/// A chunk of decoded audio.
#[derive(Debug, Clone, Copy)]
pub struct Chunk<'a> {
    /// Interleaved samples.
    pub samples: &'a [f32],
    pub rate: u32,
    pub channels: usize,
}

impl TrackDecoder {
    /// Probe the container and set up a decoder for the first audio track.
    ///
    /// `hint` is a file extension (`"mp3"`) or MIME type (`"audio/flac"`) that speeds up probing.
    pub fn open(mut source: Box<dyn MediaSource>, hint: Option<&str>) -> Result<Self, Error> {
        let edit = if source.is_seekable() {
            mp4edit::read_edit(&mut *source)
        } else {
            None
        };
        let mss = MediaSourceStream::new(source, Default::default());
        let mut h = Hint::new();
        if let Some(x) = hint {
            if x.contains('/') {
                h.mime_type(x);
            } else {
                h.with_extension(x);
            }
        }
        let format = symphonia::default::get_probe()
            .probe(
                &h,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| Error::Decode(format!("unsupported or corrupt stream: {e}")))?;
        Self::from_format(format, edit)
    }

    fn from_format(
        mut format: Box<dyn FormatReader>,
        edit: Option<mp4edit::Edit>,
    ) -> Result<Self, Error> {
        let replay_gain = tags_replay_gain(&mut *format);
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| Error::Decode("no audio track".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| Error::Decode("audio track without codec parameters".into()))?
            .clone();
        let track_id = track.id;
        let time_base = track.time_base;
        let delay = u64::from(track.delay.unwrap_or(0));
        let duration = match (track.num_frames, time_base, track.duration) {
            (_, Some(tb), Some(d)) => tb.calc_duration(d).map(|t| t.as_secs_f64()),
            (Some(n), _, _) => params.sample_rate.map(|r| n as f64 / f64::from(r)),
            _ => None,
        };
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2);
        let mut duration = duration;
        let edit = match (edit, params.sample_rate) {
            (Some(e), Some(rate)) if e.timescale > 0 => {
                let to_frames =
                    |t: u64| (u128::from(t) * u128::from(rate) / u128::from(e.timescale)) as u64;
                let start = to_frames(e.start);
                let len = e.duration.map(to_frames);
                if let Some(l) = len {
                    duration = Some(l as f64 / f64::from(rate));
                }
                Some((start, len.map(|l| start + l)))
            }
            _ => None,
        };
        let is_opus = params.codec == CODEC_ID_OPUS;
        let (codec, rate) = if is_opus {
            if channels == 0 || channels > 2 {
                return Err(Error::Decode(format!(
                    "Opus with {channels} channels is not supported"
                )));
            }
            let dec = OpusDecoder::new(OPUS_RATE, channels)
                .map_err(|e| Error::Decode(format!("opus: {e:?}")))?;
            (Codec::Opus(Box::new(dec)), OPUS_RATE)
        } else {
            let dec = symphonia::default::get_codecs()
                .make_audio_decoder(&params, &AudioDecoderOptions::default())
                .map_err(|e| Error::Decode(format!("unsupported codec: {e}")))?;
            (Codec::Symphonia(dec), params.sample_rate.unwrap_or(44_100))
        };
        Ok(Self {
            format,
            codec,
            track_id,
            time_base,
            rate,
            channels: channels.max(1),
            duration,
            buf: Vec::new(),
            // Symphonia trims encoder delay for its own codecs; Opus pre-skip is ours to drop.
            skip_frames: if is_opus { delay } else { 0 },
            edit,
            frame_pos: 0,
            window_start: edit.map_or(0, |(s, _)| s),
            replay_gain,
            eof: false,
        })
    }

    /// Sample rate of the decoded audio (may be refined after the first chunk).
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Channel count of the decoded audio (may be refined after the first chunk).
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Track duration in seconds, if the container knows it.
    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    /// ReplayGain values from the file's tags.
    pub fn replay_gain(&self) -> Option<ReplayGain> {
        self.replay_gain
    }

    /// Whether the end of the stream was reached.
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Decode the next chunk. `Ok(None)` at the end of the stream.
    pub fn next_chunk(&mut self) -> Result<Option<Chunk<'_>>, Error> {
        loop {
            if self.eof {
                return Ok(None);
            }
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // Truncated file: play what we have.
                    self.eof = true;
                    return Ok(None);
                }
                Err(SymError::ResetRequired) => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(e) => return Err(Error::Decode(format!("demux: {e}"))),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            self.buf.clear();
            match &mut self.codec {
                Codec::Symphonia(dec) => match dec.decode(&packet) {
                    Ok(audio) => {
                        self.rate = audio.spec().rate();
                        self.channels = audio.spec().channels().count().max(1);
                        audio.copy_to_vec_interleaved(&mut self.buf);
                    }
                    Err(SymError::DecodeError(e)) => {
                        tracing::debug!("skipping corrupt packet: {e}");
                        continue;
                    }
                    Err(SymError::IoError(e)) => {
                        tracing::debug!("I/O error inside packet: {e}");
                        continue;
                    }
                    Err(e) => return Err(Error::Decode(format!("decode: {e}"))),
                },
                Codec::Opus(dec) => {
                    let ch = self.channels;
                    self.buf.resize(OPUS_MAX_FRAME * ch, 0.0);
                    // The Opus decoder is young; never let a malformed packet take down the
                    // player. A panic is treated like a corrupt packet.
                    let buf = &mut self.buf;
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        dec.decode_float(&packet.data, buf, false)
                    }));
                    let frames = match res {
                        Ok(Ok(n)) => n,
                        Ok(Err(e)) => {
                            tracing::debug!("skipping corrupt opus packet: {e:?}");
                            continue;
                        }
                        Err(_) => {
                            tracing::warn!("opus decoder panicked; resetting it");
                            dec.reset();
                            continue;
                        }
                    };
                    self.buf.truncate(frames * ch);
                    let start = usize::try_from(packet.trim_start.get())
                        .unwrap_or(usize::MAX)
                        .min(frames);
                    let end = usize::try_from(packet.trim_end.get())
                        .unwrap_or(usize::MAX)
                        .min(frames - start);
                    self.buf.truncate((frames - end) * ch);
                    self.buf.drain(..start * ch);
                }
            }
            if let Some((_, end)) = self.edit {
                let ch = self.channels;
                let n = (self.buf.len() / ch) as u64;
                let first = self.frame_pos;
                self.frame_pos += n;
                let keep_from = self.window_start.saturating_sub(first).min(n);
                let keep_to = end.map_or(n, |e| e.saturating_sub(first).min(n));
                if keep_to <= keep_from {
                    if end.is_some_and(|e| self.frame_pos >= e) {
                        self.eof = true;
                        return Ok(None);
                    }
                    continue;
                }
                self.buf.truncate(keep_to as usize * ch);
                self.buf.drain(..keep_from as usize * ch);
            } else if self.skip_frames > 0 {
                let frames = (self.buf.len() / self.channels) as u64;
                let skip = self.skip_frames.min(frames);
                self.buf
                    .drain(..usize::try_from(skip).unwrap_or(0) * self.channels);
                self.skip_frames -= skip;
            }
            if self.buf.is_empty() {
                continue;
            }
            return Ok(Some(Chunk {
                samples: &self.buf,
                rate: self.rate,
                channels: self.channels,
            }));
        }
    }

    /// Seek to `secs`. Returns the position actually reached (sample accurate when possible).
    pub fn seek(&mut self, secs: f64) -> Result<f64, Error> {
        let secs = secs.max(0.0);
        // With an edit list the codec timeline starts before the audible part.
        let offset = self
            .edit
            .map_or(0.0, |(s, _)| s as f64 / f64::from(self.rate.max(1)));
        let time = Time::try_from_secs_f64(secs + offset)
            .ok_or_else(|| Error::Decode(format!("invalid seek position {secs}")))?;
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| Error::Decode(format!("seek: {e}")))?;
        match &mut self.codec {
            Codec::Symphonia(d) => d.reset(),
            Codec::Opus(d) => d.reset(),
        }
        self.eof = false;
        if self.edit.is_some() {
            let frames = |ts| match self.time_base {
                Some(tb) => (tb.calc_time_saturating(ts).as_secs_f64() * f64::from(self.rate))
                    .round() as u64,
                None => 0,
            };
            self.frame_pos = frames(seeked.actual_ts);
            self.window_start = frames(seeked.required_ts);
            self.skip_frames = 0;
            return Ok(secs);
        }
        let delta = seeked.required_ts.get() - seeked.actual_ts.get();
        self.skip_frames = 0;
        if delta > 0
            && let Some(tb) = self.time_base
        {
            let dt = tb.calc_time_saturating(seeked.required_ts).as_secs_f64()
                - tb.calc_time_saturating(seeked.actual_ts).as_secs_f64();
            self.skip_frames = (dt * f64::from(self.rate)).round().max(0.0) as u64;
        }
        Ok(secs)
    }
}

/// ReplayGain values from the container's tags.
fn tags_replay_gain(format: &mut dyn FormatReader) -> Option<ReplayGain> {
    let mut md = format.metadata();
    let mut pairs: Vec<(String, String)> = Vec::new();
    // Tags from ID3 before the stream and from the container end up in separate revisions.
    loop {
        if let Some(rev) = md.current() {
            let tags = rev
                .media
                .tags
                .iter()
                .chain(rev.per_track.iter().flat_map(|t| t.metadata.tags.iter()));
            pairs.extend(tags.filter_map(tag_pair));
        }
        if md.pop().is_none() {
            break;
        }
    }
    ReplayGain::from_tags(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
}

fn tag_pair(tag: &Tag) -> Option<(String, String)> {
    let std = match &tag.std {
        Some(StandardTag::ReplayGainTrackGain(v)) => Some(("replaygain_track_gain", v)),
        Some(StandardTag::ReplayGainTrackPeak(v)) => Some(("replaygain_track_peak", v)),
        Some(StandardTag::ReplayGainAlbumGain(v)) => Some(("replaygain_album_gain", v)),
        Some(StandardTag::ReplayGainAlbumPeak(v)) => Some(("replaygain_album_peak", v)),
        _ => None,
    };
    if let Some((k, v)) = std {
        return Some((k.to_string(), v.to_string()));
    }
    let value = match &tag.raw.value {
        RawValue::String(s) => s.to_string(),
        RawValue::StringList(l) => l.first()?.clone(),
        RawValue::SignedInt(i) => i.to_string(),
        RawValue::Float(f) => f.to_string(),
        _ => return None,
    };
    // MP4 freeform keys look like `----:com.apple.iTunes:REPLAYGAIN_TRACK_GAIN`.
    let key = tag.raw.key.rsplit(':').next().unwrap_or(&tag.raw.key);
    Some((key.to_string(), value))
}
