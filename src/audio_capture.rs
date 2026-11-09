//! Silent, real-time offline output. IEEE float stereo WAV, unclipped for analysis.
use rodio::Source;
use std::{
    fs::File,
    io::{BufWriter, Seek, SeekFrom, Write},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
pub fn run(
    path: std::path::PathBuf,
    mut source: impl Source,
    stop: Receiver<()>,
) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    f.write_all(&[0; 44])?;
    let start = Instant::now();
    let mut frames = 0u32;
    loop {
        for _ in 0..480 {
            for _ in 0..2 {
                f.write_all(&source.next().unwrap_or(0.).to_le_bytes())?;
            }
            frames += 1;
        }
        if stop
            .try_recv()
            .is_err_and(|e| matches!(e, std::sync::mpsc::TryRecvError::Disconnected))
        {
            break;
        }
        let deadline = start + Duration::from_secs_f64(frames as f64 / 48000.);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let n = frames * 8;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(b"RIFF")?;
    f.write_all(&(n + 36).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&48000u32.to_le_bytes())?;
    f.write_all(&384000u32.to_le_bytes())?;
    f.write_all(&8u16.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&n.to_le_bytes())?;
    f.flush()
}
