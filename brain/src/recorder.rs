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

/// One column per ControlState field, preceded by the wall clock at sample time.
fn build_schema() -> Arc<Schema> {
    let f32f = |name: &str| Field::new(name, DataType::Float32, false);
    Arc::new(Schema::new(vec![
        Field::new("wall_ms", DataType::Int64, false),
        Field::new("seq", DataType::UInt64, false),
        Field::new("timestamp_us", DataType::UInt64, false),
        f32f("energy"),
        f32f("energy_low"),
        f32f("energy_mid"),
        f32f("energy_high"),
        f32f("energy_slow"),
        f32f("bass_ratio"),
        f32f("tilt"),
        f32f("crest"),
        f32f("rms_var"),
        f32f("onset_strength"),
        Field::new("onset_count", DataType::UInt64, false),
        f32f("last_onset_strength"),
        f32f("onset_density"),
        f32f("centroid"),
        f32f("spread"),
        f32f("flatness"),
        f32f("rolloff"),
        f32f("spectral_flux"),
        f32f("bpm"),
        f32f("tempo_confidence"),
        f32f("beat_phase"),
        f32f("energy_3min"),
        f32f("quiet_seconds"),
        f32f("music_amount"),
        Field::new("state", DataType::UInt8, false),
        f32f("noise_floor"),
        f32f("agc_ref"),
        Field::new("xrun_count", DataType::UInt64, false),
    ]))
}

fn to_batch(schema: &Arc<Schema>, rows: &[Row]) -> Result<RecordBatch, Box<dyn Error>> {
    let f32c = |get: &dyn Fn(&ControlState) -> f32| -> ArrayRef {
        Arc::new(Float32Array::from(
            rows.iter().map(|r| get(&r.cs)).collect::<Vec<_>>(),
        ))
    };
    let u64c = |get: &dyn Fn(&ControlState) -> u64| -> ArrayRef {
        Arc::new(UInt64Array::from(
            rows.iter().map(|r| get(&r.cs)).collect::<Vec<_>>(),
        ))
    };
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(
            rows.iter().map(|r| r.wall_ms).collect::<Vec<_>>(),
        )),
        u64c(&|c| c.seq),
        u64c(&|c| c.timestamp_us),
        f32c(&|c| c.energy),
        f32c(&|c| c.energy_low),
        f32c(&|c| c.energy_mid),
        f32c(&|c| c.energy_high),
        f32c(&|c| c.energy_slow),
        f32c(&|c| c.bass_ratio),
        f32c(&|c| c.tilt),
        f32c(&|c| c.crest),
        f32c(&|c| c.rms_var),
        f32c(&|c| c.onset_strength),
        u64c(&|c| c.onset_count),
        f32c(&|c| c.last_onset_strength),
        f32c(&|c| c.onset_density),
        f32c(&|c| c.centroid),
        f32c(&|c| c.spread),
        f32c(&|c| c.flatness),
        f32c(&|c| c.rolloff),
        f32c(&|c| c.spectral_flux),
        f32c(&|c| c.bpm),
        f32c(&|c| c.tempo_confidence),
        f32c(&|c| c.beat_phase),
        f32c(&|c| c.energy_3min),
        f32c(&|c| c.quiet_seconds),
        f32c(&|c| c.music_amount),
        Arc::new(UInt8Array::from(
            rows.iter().map(|r| r.cs.state).collect::<Vec<_>>(),
        )),
        f32c(&|c| c.noise_floor),
        f32c(&|c| c.agc_ref),
        u64c(&|c| c.xrun_count),
    ];
    Ok(RecordBatch::try_new(schema.clone(), columns)?)
}
