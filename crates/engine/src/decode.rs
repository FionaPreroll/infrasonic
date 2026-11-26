// SPDX-License-Identifier: GPL-3.0-only
//! Demuxing and decoding with Symphonia, plus a pure Rust Opus decoder.

use opus_decoder::OpusDecoder;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

use crate::Error;

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
    pub fn open(source: Box<dyn MediaSource>, hint: Option<&str>) -> Result<Self, Error> {
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
        Self::from_format(format)
    }

    fn from_format(format: Box<dyn FormatReader>) -> Result<Self, Error> {
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
            if self.skip_frames > 0 {
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
        let time = Time::try_from_secs_f64(secs)
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
        let delta = seeked.required_ts.get() - seeked.actual_ts.get();
        self.skip_frames = 0;
        if delta > 0 {
            if let Some(tb) = self.time_base {
                let dt = tb.calc_time_saturating(seeked.required_ts).as_secs_f64()
                    - tb.calc_time_saturating(seeked.actual_ts).as_secs_f64();
                self.skip_frames = (dt * f64::from(self.rate)).round().max(0.0) as u64;
            }
        }
        Ok(secs)
    }
}
