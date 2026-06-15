//! Sound-profile recorder (SOUND.md §14): a dedicated OS thread samples the
//! ControlState snapshot at 10 Hz and writes it to Arrow/Parquet files for
//! off-line analysis. All blocking file I/O lives on this thread, so the DMX
//! loop is never disturbed. Files rotate every 10 minutes.

use crate::config as cfg;
use crate::control::{ControlReader, ControlState};
use arrow::array::{ArrayRef, Float32Array, Int64Array, UInt8Array, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::Utc;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::error::Error;
use std::fs::File;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Row {
    wall_ms: i64,
    cs: ControlState,
}

/// Spawns the recorder on its own thread and returns immediately. The thread
/// never dies: errors are logged and retried with a fresh file.
pub fn spawn_recorder(reader: ControlReader) -> JoinHandle<()> {
    thread::Builder::new()
        .name("recorder".into())
        .spawn(move || {
            let schema = build_schema();
            loop {
                if let Err(e) = record_one_file(&reader, &schema) {
                    eprintln!("recorder: error: {e}");
                    eprintln!("recorder: retrying in {}s", cfg::RECORDER_RETRY_S);
                    thread::sleep(Duration::from_secs(cfg::RECORDER_RETRY_S));
                }
            }
        })
        .expect("failed to spawn recorder thread")
}

/// Samples at RECORDER_RATE_HZ into one Parquet file, flushing a row group
/// every RECORDER_BATCH_ROWS rows, then closes it after RECORDER_ROTATE_S.
fn record_one_file(reader: &ControlReader, schema: &Arc<Schema>) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(cfg::RECORDER_DIR)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = format!("{}/memory.{stamp}.parquet", cfg::RECORDER_DIR);
    let file = File::create(&path)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;
    eprintln!("recorder: writing {path}");

    let tick = Duration::from_millis(1_000 / cfg::RECORDER_RATE_HZ);
    let opened = Instant::now();
    let mut next = Instant::now() + tick;
    let mut rows: Vec<Row> = Vec::with_capacity(cfg::RECORDER_BATCH_ROWS);

    while opened.elapsed().as_secs() < cfg::RECORDER_ROTATE_S {
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        }
        next += tick;

        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        rows.push(Row {
            wall_ms,
            cs: reader.snapshot(),
        });

        if rows.len() >= cfg::RECORDER_BATCH_ROWS {
            writer.write(&to_batch(schema, &rows)?)?;
            writer.flush()?;
            rows.clear();
        }
    }

    if !rows.is_empty() {
        writer.write(&to_batch(schema, &rows)?)?;
    }
    writer.close()?;
    eprintln!("recorder: rotated {path}");
    Ok(())
}

/// Single source of truth for the recorder's columns: each row is the Parquet
/// column name, its Arrow `DataType`, the concrete Arrow array type, and the
/// accessor from a sampled `Row`. `build_schema` and `to_batch` are generated
/// from the same list, so a new field can never desync the schema from the data.
macro_rules! columns {
    ($($name:literal, $dtype:ident, $array:ty, $get:expr);+ $(;)?) => {
        /// One column per ControlState field, preceded by the wall clock at sample time.
        fn build_schema() -> Arc<Schema> {
            Arc::new(Schema::new(vec![
                $( Field::new($name, DataType::$dtype, false), )+
            ]))
        }

        fn to_batch(schema: &Arc<Schema>, rows: &[Row]) -> Result<RecordBatch, Box<dyn Error>> {
            let columns: Vec<ArrayRef> = vec![
                $( Arc::new(<$array>::from(
                    rows.iter().map($get).collect::<Vec<_>>(),
                )) as ArrayRef, )+
            ];
            Ok(RecordBatch::try_new(schema.clone(), columns)?)
        }
    };
}

columns! {
    "wall_ms",            Int64,   Int64Array,   |r: &Row| r.wall_ms;
    "seq",                UInt64,  UInt64Array,  |r: &Row| r.cs.seq;
    "timestamp_us",       UInt64,  UInt64Array,  |r: &Row| r.cs.timestamp_us;
    "energy",             Float32, Float32Array, |r: &Row| r.cs.energy;
    "energy_low",         Float32, Float32Array, |r: &Row| r.cs.energy_low;
    "energy_mid",         Float32, Float32Array, |r: &Row| r.cs.energy_mid;
    "energy_high",        Float32, Float32Array, |r: &Row| r.cs.energy_high;
    "energy_slow",        Float32, Float32Array, |r: &Row| r.cs.energy_slow;
    "bass_ratio",         Float32, Float32Array, |r: &Row| r.cs.bass_ratio;
    "tilt",               Float32, Float32Array, |r: &Row| r.cs.tilt;
    "crest",              Float32, Float32Array, |r: &Row| r.cs.crest;
    "rms_var",            Float32, Float32Array, |r: &Row| r.cs.rms_var;
    "onset_strength",     Float32, Float32Array, |r: &Row| r.cs.onset_strength;
    "onset_count",        UInt64,  UInt64Array,  |r: &Row| r.cs.onset_count;
    "last_onset_strength", Float32, Float32Array, |r: &Row| r.cs.last_onset_strength;
    "onset_density",      Float32, Float32Array, |r: &Row| r.cs.onset_density;
    "centroid",           Float32, Float32Array, |r: &Row| r.cs.centroid;
    "spread",             Float32, Float32Array, |r: &Row| r.cs.spread;
    "flatness",           Float32, Float32Array, |r: &Row| r.cs.flatness;
    "rolloff",            Float32, Float32Array, |r: &Row| r.cs.rolloff;
    "spectral_flux",      Float32, Float32Array, |r: &Row| r.cs.spectral_flux;
    "bpm",                Float32, Float32Array, |r: &Row| r.cs.bpm;
    "tempo_confidence",   Float32, Float32Array, |r: &Row| r.cs.tempo_confidence;
    "beat_phase",         Float32, Float32Array, |r: &Row| r.cs.beat_phase;
    "energy_3min",        Float32, Float32Array, |r: &Row| r.cs.energy_3min;
    "quiet_seconds",      Float32, Float32Array, |r: &Row| r.cs.quiet_seconds;
    "music_amount",       Float32, Float32Array, |r: &Row| r.cs.music_amount;
    "state",              UInt8,   UInt8Array,   |r: &Row| r.cs.state;
    "noise_floor",        Float32, Float32Array, |r: &Row| r.cs.noise_floor;
    "agc_ref",            Float32, Float32Array, |r: &Row| r.cs.agc_ref;
    "xrun_count",         UInt64,  UInt64Array,  |r: &Row| r.cs.xrun_count;
}
