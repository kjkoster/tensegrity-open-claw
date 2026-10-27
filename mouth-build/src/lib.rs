//! Compiles a rig's recordings into it, from the files as they were found.
//!
//! The rig crate's build script hands [`sounds`] its sound table and the frame rate, and gets
//! a `sounds` module in `OUT_DIR` that its `main.rs` includes: one `mouth::Sound` per
//! recording, and all of them together as `SOUNDS`, which is what the mouth is started with.
//! Nothing is read from disk at run time, and there is no decoder in the running program.
//!
//! Each file is decoded whatever it is — AAC in an `.m4a`, MP3 — and turned into the one form
//! the mouth plays: stereo, 16-bit, at [`SAMPLE_RATE_HZ`]. On the way it is trimmed of what
//! cannot be heard at either end, so its first sample is the sound and a cue lands on it, and
//! brought up to full scale and then down by the table's gain, so the samples are stored at
//! the level they play at and the mouth only adds them up.
//!
//! Beside the samples goes the recording's envelope: its loudness, one byte per DMX frame, at
//! the frame rate the rig runs at. The samples are padded with silence to a whole number of
//! frames, so the envelope's length is the recording's length in frames, exactly, and a show
//! that plays the envelope from the frame it cued the sound is in step with it for the whole
//! recording. A show that changes the frame rate gets envelopes at the new rate from the next
//! build.
//!
//! Every problem with a file panics, naming it: a build script that panics fails the build,
//! which is the earliest place to hear about a recording the mouth cannot play.

use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Fft, FixedSync, Resampler};
use std::env;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// The rate every recording is stored and played at: the audio interface's native rate, so
/// the playback opens the interface at it and nothing resamples at run time.
pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// What the mouth plays: two speakers, a channel each.
const CHANNELS: usize = 2;

/// How far below a recording's peak a sample counts as inaudible, for trimming its ends. At
/// 60 dB down, the step left where the trim cuts is as inaudible as what it took away, so the
/// cut needs no fade.
const TRIM_BELOW_PEAK_DB: f64 = -60.0;

/// Where a recording's loudest sample lands. A hair under full scale rather than on it: the
/// interface's reconstruction filter can swing between two samples past the larger of them,
/// and a recording normalised to exactly full scale would clip there.
const PEAK_DBFS: f64 = -0.3;

/// The chunk the resampler works in. It rounds this up to a size that suits the pair of rates;
/// any few hundred to a few thousand frames gives the same result for a whole file.
const RESAMPLER_CHUNK_FRAMES: usize = 1024;

/// Compiles every recording in `table` — a file in `dir` and its gain in dB — into the
/// `sounds` module, with envelopes at `frame_rate_hz`.
///
/// A sound is named by its file's stem, so `heartbeat.m4a` becomes `sounds::HEARTBEAT`.
pub fn sounds(dir: &Path, table: &[(&str, f64)], frame_rate_hz: u32) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // Cargo scans a directory for changes to anything in it, so this covers every recording.
    println!("cargo::rerun-if-changed={}", dir.display());
    if table.is_empty() {
        panic!("the sound table is empty: a rig without sounds leaves the mouth out");
    }

    let mut out = String::new();
    writeln!(out, "pub mod sounds {{").unwrap();
    writeln!(out, "    use mouth::{{Sound, Sounds}};").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    pub const COUNT: usize = {};", table.len()).unwrap();

    let mut idents: Vec<String> = Vec::new();
    for (index, &(file, gain_db)) in table.iter().enumerate() {
        let path = dir.join(file);
        let name = name_of(&path);
        let ident = name.to_ascii_uppercase();
        if idents.contains(&ident) {
            fail(&path, "names the same sound as another file in the table");
        }

        if gain_db > 0.0 {
            fail(
                &path,
                &format!(
                    "has a gain of {gain_db} dB; a recording is compiled at full scale, so 0 dB \
                     is the loudest it plays"
                ),
            );
        }
        let recording = Recording::compile(&path, gain_db, frame_rate_hz);
        let samples_file = format!("{name}.pcm");
        let envelope_file = format!("{name}.envelope");
        fs::write(out_dir.join(&samples_file), recording.bytes()).unwrap();
        fs::write(out_dir.join(&envelope_file), &recording.envelope).unwrap();

        writeln!(out).unwrap();
        writeln!(out, "    pub static {ident}: Sound = Sound {{").unwrap();
        writeln!(out, "        name: {name:?},").unwrap();
        writeln!(out, "        index: {index},").unwrap();
        writeln!(
            out,
            "        frames: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{samples_file}\")).as_chunks::<4>().0,"
        )
        .unwrap();
        writeln!(
            out,
            "        envelope: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{envelope_file}\")),"
        )
        .unwrap();
        writeln!(out, "    }};").unwrap();
        idents.push(ident);
    }

    let all: Vec<String> = idents.iter().map(|ident| format!("&{ident}")).collect();
    writeln!(out).unwrap();
    writeln!(out, "    pub static SOUNDS: Sounds<COUNT> = Sounds {{").unwrap();
    writeln!(out, "        rate_hz: {SAMPLE_RATE_HZ},").unwrap();
    writeln!(out, "        all: [{}],", all.join(", ")).unwrap();
    writeln!(out, "    }};").unwrap();
    writeln!(out, "}}").unwrap();
    fs::write(out_dir.join("sounds.rs"), out).unwrap();
}

/// One recording, ready to compile in.
struct Recording {
    /// Stereo frames at [`SAMPLE_RATE_HZ`], a whole number of DMX frames long, at their gain.
    frames: Vec<[i16; CHANNELS]>,
    /// The loudness of each DMX frame's worth of samples, the loudest at 255.
    envelope: Vec<u8>,
}

impl Recording {
    fn compile(path: &Path, gain_db: f64, frame_rate_hz: u32) -> Recording {
        let (rate, frames) = decode(path);
        let frames = resample(path, rate, frames);
        let peak = frames
            .iter()
            .map(|&frame| loudest(frame))
            .fold(0.0, f64::max);
        if peak == 0.0 {
            fail(path, "is silent");
        }

        let mut frames = trim(frames, peak);
        let slices = dmx_frames(frames.len(), frame_rate_hz);
        frames.resize(slice_start(slices, frame_rate_hz), [0.0; CHANNELS]);
        let envelope = envelope(&frames, slices, frame_rate_hz);

        // Up to full scale, then down by the gain, in one step, so the samples are rounded
        // once.
        let scale = 10f64.powf((PEAK_DBFS + gain_db) / 20.0) / peak * f64::from(i16::MAX);
        let frames = frames
            .iter()
            .map(|frame| frame.map(|sample| (sample * scale).round() as i16))
            .collect();
        Recording { frames, envelope }
    }

    /// The samples as the mouth reads them: each frame left then right, each little-endian.
    fn bytes(&self) -> Vec<u8> {
        self.frames
            .iter()
            .flatten()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }
}

/// The file's sample rate, and its samples as stereo frames. A mono file is played on both
/// speakers.
fn decode(path: &Path) -> (u32, Vec<[f64; CHANNELS]>) {
    let file = File::open(path).unwrap_or_else(|e| fail(path, &format!("cannot be opened: {e}")));
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .unwrap_or_else(|e| fail(path, &format!("is not in a format the build reads: {e}")));

    let (track_id, mut decoder) = {
        let track = format
            .default_track(TrackType::Audio)
            .unwrap_or_else(|| fail(path, "has no audio track"));
        let params = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .unwrap_or_else(|| fail(path, "does not say how its audio is encoded"));
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .unwrap_or_else(|e| fail(path, &format!("is in a codec the build cannot decode: {e}")));
        (track.id, decoder)
    };

    let mut rate = None;
    let mut planes: Vec<Vec<f64>> = Vec::new();
    let mut frames = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(e) => fail(path, &format!("cannot be read: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buffer = decoder
            .decode(&packet)
            .unwrap_or_else(|e| fail(path, &format!("cannot be decoded: {e}")));
        let packet_rate = buffer.spec().rate();
        if *rate.get_or_insert(packet_rate) != packet_rate {
            fail(path, "changes its sample rate partway through");
        }
        buffer.copy_to_vecs_planar::<f64>(&mut planes);
        match planes.as_slice() {
            [mono] => frames.extend(mono.iter().map(|&sample| [sample, sample])),
            [left, right] => frames.extend(left.iter().zip(right).map(|(&l, &r)| [l, r])),
            _ => fail(
                path,
                &format!(
                    "has {} channels, and the mouth plays mono or stereo",
                    planes.len()
                ),
            ),
        }
    }
    let rate = rate.unwrap_or_else(|| fail(path, "holds no audio"));
    (rate, frames)
}

/// The frames at [`SAMPLE_RATE_HZ`].
fn resample(path: &Path, rate: u32, frames: Vec<[f64; CHANNELS]>) -> Vec<[f64; CHANNELS]> {
    if rate == SAMPLE_RATE_HZ {
        return frames;
    }
    let count = frames.len();
    let input = InterleavedOwned::new_from(frames.into_flattened(), CHANNELS, count).unwrap();
    let mut resampler = Fft::<f64>::new(
        rate as usize,
        SAMPLE_RATE_HZ as usize,
        RESAMPLER_CHUNK_FRAMES,
        CHANNELS,
        FixedSync::Both,
    )
    .unwrap_or_else(|e| fail(path, &format!("cannot be resampled from {rate} Hz: {e}")));
    let output = resampler
        .process_all(&input, count, None)
        .unwrap_or_else(|e| fail(path, &format!("cannot be resampled from {rate} Hz: {e}")));
    output.take_data().as_chunks::<CHANNELS>().0.to_vec()
}

/// The frames from the first audible one to the last, judged by whichever speaker is louder.
fn trim(frames: Vec<[f64; CHANNELS]>, peak: f64) -> Vec<[f64; CHANNELS]> {
    let threshold = peak * 10f64.powf(TRIM_BELOW_PEAK_DB / 20.0);
    let audible = |frame: &[f64; CHANNELS]| loudest(*frame) > threshold;
    // The peak itself is audible, so both ends exist.
    let first = frames.iter().position(audible).unwrap();
    let last = frames.iter().rposition(audible).unwrap();
    frames[first..=last].to_vec()
}

fn loudest(frame: [f64; CHANNELS]) -> f64 {
    frame
        .iter()
        .fold(0.0, |loudest, sample| sample.abs().max(loudest))
}

/// How many DMX frames it takes to cover `frames` samples: the last one may be partly
/// silence.
fn dmx_frames(frames: usize, frame_rate_hz: u32) -> usize {
    (frames as u64 * u64::from(frame_rate_hz)).div_ceil(u64::from(SAMPLE_RATE_HZ)) as usize
}

/// The first sample of DMX frame `slice`. Computed from the two rates for every frame rather
/// than stepped by a fixed count, so a frame rate that does not divide the sample rate still
/// lands every frame on the sample nearest its start, and the error never adds up.
fn slice_start(slice: usize, frame_rate_hz: u32) -> usize {
    (slice as u64 * u64::from(SAMPLE_RATE_HZ) / u64::from(frame_rate_hz)) as usize
}

/// The RMS of each DMX frame's worth of samples, over both speakers, the loudest at 255.
///
/// Linear in amplitude, so the lamp follows the sound's shape the way the waveform draws it.
fn envelope(frames: &[[f64; CHANNELS]], slices: usize, frame_rate_hz: u32) -> Vec<u8> {
    let levels: Vec<f64> = (0..slices)
        .map(|slice| {
            let span =
                &frames[slice_start(slice, frame_rate_hz)..slice_start(slice + 1, frame_rate_hz)];
            let energy: f64 = span.iter().flatten().map(|sample| sample * sample).sum();
            (energy / (span.len() * CHANNELS) as f64).sqrt()
        })
        .collect();
    let loudest = levels.iter().copied().fold(0.0, f64::max);
    levels
        .iter()
        .map(|level| (level / loudest * f64::from(u8::MAX)).round() as u8)
        .collect()
}

/// The sound's name, from its file's stem, which must make a Rust identifier once uppercased.
fn name_of(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let valid = stem.starts_with(|c: char| c.is_ascii_lowercase())
        && stem
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        fail(
            path,
            "must be named in lower case, digits and underscores, starting with a letter, \
             because its name becomes a constant",
        );
    }
    stem.to_string()
}

fn fail(path: &Path, problem: &str) -> ! {
    panic!("{}: {problem}", path.display())
}
