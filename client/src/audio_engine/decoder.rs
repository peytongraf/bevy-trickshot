//! A kira [`Sound`] that decodes a clip's compressed bytes as it plays,
//! packet by packet on the audio thread (what rodio, under bevy_audio, did
//! before). kira's own `StaticSoundData` would hold every clip fully decoded
//! as 32-bit stereo instead — with ~575 clips (mostly operator quotes, all
//! loaded at startup) that's ~840 MB of RAM against ~145 MB of mp3/wav/ogg
//! bytes — and its `StreamingSoundData` spins up a decoder thread per sound,
//! too heavy for footsteps and zombie moans.
//!
//! Volume, pause and stop are driven through [`Playback`]'s atomics,
//! so the Bevy side can poke them each frame without a command queue.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use kira::info::Info;
use kira::sound::{Sound, SoundData};
use kira::{interpolate_frame, Frame};
use symphonia::core::audio::conv::{FromSample, IntoSample};
use symphonia::core::audio::sample::Sample;
use symphonia::core::audio::{Audio, AudioBuffer, GenericAudioBufferRef};
use symphonia::core::codecs::audio::AudioDecoder;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::units::{Time, Timestamp};

/// A clip opened for decoding from the start: its format reader and decoder.
pub(super) struct OpenClip {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    sample_rate: u32,
    /// How many frames long it is, when its header says.
    frames: Option<u64>,
}

/// Open `bytes` (a whole mp3 / wav / ogg / flac file) for decoding. Cheap —
/// it only reads the headers — so it's done fresh for every play, and once
/// up front by the asset loader to reject a file that can't be decoded.
pub(super) fn open(bytes: &Arc<[u8]>) -> Result<OpenClip, SymphoniaError> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.clone())), Default::default());
    let reader = symphonia::default::get_probe().probe(
        &Hint::new(),
        source,
        Default::default(),
        Default::default(),
    )?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or(SymphoniaError::Unsupported("no audio track"))?;
    let Some(CodecParameters::Audio(params)) = track.codec_params.as_ref() else {
        return Err(SymphoniaError::Unsupported("no audio track"));
    };
    let sample_rate = params
        .sample_rate
        .ok_or(SymphoniaError::Unsupported("unknown sample rate"))?;
    let decoder = symphonia::default::get_codecs().make_audio_decoder(params, &Default::default())?;
    let track_id = track.id;
    let frames = track.num_frames;
    Ok(OpenClip {
        reader,
        decoder,
        track_id,
        sample_rate,
        frames,
    })
}

impl OpenClip {
    /// Start `fraction` (0..1) of the way in rather than at the top. Does
    /// nothing for a clip whose length isn't known.
    pub(super) fn skip_to(&mut self, fraction: f32) {
        let Some(frames) = self.frames.filter(|&f| f > 0) else {
            return;
        };
        let secs = frames as f64 * fraction.clamp(0.0, 1.0) as f64 / self.sample_rate as f64;
        let Some(time) = Time::try_from_secs_f64(secs) else {
            return;
        };
        let seek = self.reader.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track_id),
            },
        );
        if seek.is_ok() {
            self.decoder.reset();
        }
    }
}

/// The controls and state one playing sound shares between the game (its
/// `AudioSink` / `SpatialAudioSink`) and the audio thread.
pub(super) struct Playback {
    /// Linear gain the sound was played / set at, as `f32` bits.
    volume: AtomicU32,
    /// Linear gain from the mix (`mix`) and master volume on top, as `f32`
    /// bits.
    mix: AtomicU32,
    /// Linear gain from a positional sound's distance (its profile), as
    /// `f32` bits; `1` for the rest.
    distance: AtomicU32,
    /// Out of earshot: held silent where it is, not decoding (looping
    /// positional sounds only).
    culled: AtomicBool,
    paused: AtomicBool,
    /// Asked to stop: fades out over one buffer, then finishes.
    stopped: AtomicBool,
    /// Done — played out, stopped, or failed to decode. Never cleared.
    finished: AtomicBool,
}

impl Playback {
    pub(super) fn new(volume: f32, mix: f32, paused: bool) -> Self {
        Self {
            volume: AtomicU32::new(volume.to_bits()),
            mix: AtomicU32::new(mix.to_bits()),
            distance: AtomicU32::new(1f32.to_bits()),
            culled: AtomicBool::new(false),
            paused: AtomicBool::new(paused),
            stopped: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    pub(super) fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub(super) fn set_volume(&self, volume: f32) {
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    pub(super) fn set_mix(&self, mix: f32) {
        self.mix.store(mix.to_bits(), Ordering::Relaxed);
    }

    pub(super) fn set_distance(&self, gain: f32) {
        self.distance.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub(super) fn set_culled(&self, culled: bool) {
        self.culled.store(culled, Ordering::Relaxed);
    }

    /// What's actually applied: the volume times the mix times the distance.
    fn gain(&self) -> f32 {
        self.volume()
            * f32::from_bits(self.mix.load(Ordering::Relaxed))
            * f32::from_bits(self.distance.load(Ordering::Relaxed))
    }

    pub(super) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub(super) fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub(super) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}

/// What gets handed to kira to play: an opened clip plus its controls.
pub(super) struct DecodingSoundData {
    pub(super) clip: OpenClip,
    /// Playback rate (`1.0` = normal; also shifts the pitch).
    pub(super) speed: f32,
    pub(super) looping: bool,
    pub(super) playback: Arc<Playback>,
}

impl SoundData for DecodingSoundData {
    type Error = ();
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn Sound>, Self::Handle), Self::Error> {
        // Gain starts right at the target — no fade-in on the first buffer.
        let gain = self.playback.gain();
        let mut sound = DecodingSound {
            // Source frames per second of output.
            rate: self.clip.sample_rate as f64 * self.speed.max(0.0) as f64,
            clip: self.clip,
            looping: self.looping,
            playback: self.playback,
            packet: Vec::new(),
            packet_pos: 0,
            decoded_since_rewind: false,
            window: [Frame::ZERO; 4],
            fraction: 0.0,
            frames_past_end: 0,
            gain,
            done: false,
        };
        // Prime the interpolation window: `[previous, current, next, next]`
        // with silence before the first frame.
        for _ in 0..3 {
            sound.advance();
        }
        Ok((Box::new(sound), ()))
    }
}

struct DecodingSound {
    clip: OpenClip,
    /// Source frames to step through per second of output: the clip's
    /// sample rate times the playback speed.
    rate: f64,
    looping: bool,
    playback: Arc<Playback>,
    /// The most recently decoded packet, and the next frame to take from it.
    packet: Vec<Frame>,
    packet_pos: usize,
    /// Whether anything's been decoded since the last loop rewind — so an
    /// empty looping clip ends instead of rewinding forever.
    decoded_since_rewind: bool,
    /// Source frames around the playhead, `[previous, current, next, next]`,
    /// for kira's 4-point interpolation.
    window: [Frame; 4],
    /// How far the playhead is between `window[1]` and `window[2]`.
    fraction: f64,
    /// Silent frames pushed into `window` since the clip ran out; it's over
    /// once the last real frame has left `window[1]`.
    frames_past_end: u8,
    /// The gain applied at the end of the last buffer (ramped toward
    /// [`Playback::gain`] across each buffer, so changes don't click).
    gain: f32,
    done: bool,
}

impl DecodingSound {
    /// Slide the window one source frame forward.
    fn advance(&mut self) {
        let next = self.next_frame().unwrap_or_else(|| {
            self.frames_past_end = self.frames_past_end.saturating_add(1);
            Frame::ZERO
        });
        self.window = [self.window[1], self.window[2], self.window[3], next];
    }

    fn next_frame(&mut self) -> Option<Frame> {
        loop {
            if let Some(&frame) = self.packet.get(self.packet_pos) {
                self.packet_pos += 1;
                return Some(frame);
            }
            if !self.decode_packet() {
                return None;
            }
        }
    }

    /// Decode the next packet into `packet`; `false` once there are none
    /// left (rewinding first when looping).
    fn decode_packet(&mut self) -> bool {
        loop {
            match self.clip.reader.next_packet() {
                Ok(Some(packet)) if packet.track_id != self.clip.track_id => continue,
                Ok(Some(packet)) => match self.clip.decoder.decode(&packet) {
                    Ok(buffer) => {
                        self.packet.clear();
                        self.packet_pos = 0;
                        if !push_frames(&buffer, &mut self.packet) {
                            return false;
                        }
                        self.decoded_since_rewind = true;
                        return true;
                    }
                    // A corrupt packet: skip it, like any player would.
                    Err(SymphoniaError::DecodeError(_)) => continue,
                    Err(_) => return false,
                },
                // The end (`Ok(None)`, or an EOF error from some readers).
                Ok(None) | Err(_) => {
                    if self.looping && self.decoded_since_rewind && self.rewind() {
                        continue;
                    }
                    return false;
                }
            }
        }
    }

    fn rewind(&mut self) -> bool {
        self.decoded_since_rewind = false;
        let seek = self.clip.reader.seek(
            SeekMode::Accurate,
            SeekTo::Timestamp {
                ts: Timestamp::new(0),
                track_id: self.clip.track_id,
            },
        );
        self.clip.decoder.reset();
        seek.is_ok()
    }

    fn finish(&mut self) {
        self.done = true;
        self.playback.finished.store(true, Ordering::Relaxed);
    }
}

impl Sound for DecodingSound {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        if self.done || out.is_empty() {
            return;
        }
        let stopping = self.playback.stopped.load(Ordering::Relaxed);
        let held = self.playback.is_paused() || self.playback.culled.load(Ordering::Relaxed);
        if held && !stopping {
            // (`out` arrives zeroed.)
            return;
        }
        let target = if stopping { 0.0 } else { self.playback.gain() };
        let step = self.rate * dt;
        let start = self.gain;
        let len = out.len() as f32;
        for (i, frame) in out.iter_mut().enumerate() {
            if self.frames_past_end >= 3 {
                break;
            }
            let [previous, current, next_1, next_2] = self.window;
            let gain = start + (target - start) * (i + 1) as f32 / len;
            *frame = interpolate_frame(previous, current, next_1, next_2, self.fraction as f32) * gain;
            self.fraction += step;
            while self.fraction >= 1.0 {
                self.fraction -= 1.0;
                self.advance();
            }
        }
        self.gain = target;
        if stopping || self.frames_past_end >= 3 {
            self.finish();
        }
    }

    fn finished(&self) -> bool {
        self.done
    }
}

/// Append `buffer`'s frames to `out`; `false` for a layout we can't play
/// (more than two channels).
fn push_frames(buffer: &GenericAudioBufferRef, out: &mut Vec<Frame>) -> bool {
    match buffer {
        GenericAudioBufferRef::U8(b) => push_planes(b, out),
        GenericAudioBufferRef::U16(b) => push_planes(b, out),
        GenericAudioBufferRef::U24(b) => push_planes(b, out),
        GenericAudioBufferRef::U32(b) => push_planes(b, out),
        GenericAudioBufferRef::S8(b) => push_planes(b, out),
        GenericAudioBufferRef::S16(b) => push_planes(b, out),
        GenericAudioBufferRef::S24(b) => push_planes(b, out),
        GenericAudioBufferRef::S32(b) => push_planes(b, out),
        GenericAudioBufferRef::F32(b) => push_planes(b, out),
        GenericAudioBufferRef::F64(b) => push_planes(b, out),
    }
}

fn push_planes<S: Sample>(buffer: &AudioBuffer<S>, out: &mut Vec<Frame>) -> bool
where
    f32: FromSample<S>,
{
    match (buffer.plane(0), buffer.plane(1), buffer.num_planes()) {
        (Some(mono), _, 1) => {
            out.extend(mono.iter().map(|&s| Frame::from_mono(s.into_sample())));
            true
        }
        (Some(left), Some(right), 2) => {
            out.extend(
                left.iter()
                    .zip(right)
                    .map(|(&l, &r)| Frame::new(l.into_sample(), r.into_sample())),
            );
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kira::info::MockInfoBuilder;

    /// Every clip under `assets/audio/` opens, and plays through to its end.
    #[test]
    fn every_clip_decodes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/audio");
        let mut dirs = vec![root];
        let mut checked = 0;
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
                if !matches!(ext, "mp3" | "wav" | "ogg" | "flac") {
                    continue;
                }
                let bytes: Arc<[u8]> = std::fs::read(&path).unwrap().into();
                let clip = open(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let playback = Arc::new(Playback::new(1.0, 1.0, false));
                let (mut sound, ()) = DecodingSoundData {
                    clip,
                    speed: 1.0,
                    looping: false,
                    playback: playback.clone(),
                }
                .into_sound()
                .unwrap();
                let info = MockInfoBuilder::new().build();
                let mut out = [Frame::ZERO; 512];
                let mut buffers = 0u64;
                while !sound.finished() {
                    sound.process(&mut out, 1.0 / 48_000.0, &info);
                    buffers += 1;
                    assert!(buffers < 48_000 * 600 / 512, "{} never ends", path.display());
                }
                assert!(playback.is_finished());
                assert!(buffers > 0, "{} is empty", path.display());
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    /// The power station's hum decodes to exactly its 8 s — the mp3's
    /// encoder delay and padding trimmed (gapless), so it loops with no gap.
    #[test]
    fn the_power_station_hum_is_gapless() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/audio/ambient/power_station_hum.mp3");
        let bytes: Arc<[u8]> = std::fs::read(&path).unwrap().into();
        let mut clip = open(&bytes).unwrap();
        let mut frames = 0usize;
        loop {
            match clip.reader.next_packet() {
                Ok(Some(packet)) if packet.track_id == clip.track_id => {
                    if let Ok(buffer) = clip.decoder.decode(&packet) {
                        frames += buffer.frames();
                    }
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert_eq!(frames, 8 * 44_100);
    }

    /// The waterfall loop knows its length, so each waterfall can start it
    /// somewhere different (`OpenClip::skip_to`).
    #[test]
    fn waterfall_loop_can_start_partway() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/audio/ambient/waterfall_loop.mp3");
        let bytes: Arc<[u8]> = std::fs::read(&path).unwrap().into();
        let mut clip = open(&bytes).unwrap();
        assert!(clip.frames.is_some_and(|f| f > 0));
        clip.skip_to(0.5);
        assert!(clip.reader.next_packet().unwrap().is_some());
    }
}
