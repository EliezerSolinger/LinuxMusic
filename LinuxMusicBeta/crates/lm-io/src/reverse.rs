//! An audio item played backwards (F-CLP-042, P-31): the stretch of a source the item plays, written in reverse as a
//! new audio resource of the project, inside an `Operation`. The original stays where it is; the reversed stretch is
//! imported like any file (hashed in `audio/`, peaks written), so two identical reversals share one file.

use std::path::Path;

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_sys_sndfile::{Existing, Reader, Sample, WriteFormat, WriteSpec, Writer};

use crate::resources::{ImportedFile, audio_folder, import_file};
use crate::take::{free_space, io_error, new_uuid};

/// Frames read or written at a time
const CHUNK: usize = 16_384;

/// Writes `frames` frames of `source` from frame `from`, in reverse order, and imports the result into `project`. The
/// file is exactly `frames` long: the frames the source lacks past its end are silence, which after the reversal
/// comes first (the item starts with its own silence). Errors: `io.reverse.disk_full`, `io.reverse.empty`, the
/// reader's, the writer's and the import's; a cancel or a failure leaves nothing behind
pub fn reverse_range(
    source: &Path,
    project: &Path,
    from: u64,
    frames: u64,
    rate: u32,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<ImportedFile> {
    progress.stage(Label::new("io.reverse.reading"));
    let mut reader = Reader::open(source)?;
    let info = *reader.info();
    let channels = info.channels;
    let from = from.min(info.frames);
    let present = frames.min(info.frames - from);
    if present == 0 {
        return Err(Error::new("io.reverse.empty").arg("path", source.display().to_string()));
    }
    let lead = frames - present;
    let folder = audio_folder(project);
    std::fs::create_dir_all(&folder).map_err(|e| io_error("io.reverse.write_failed", &folder, &e))?;
    // The temporary file and the imported copy, and a margin
    let needed = frames * channels as u64 * 4 * 2 + (2 << 20);
    if free_space(&folder)? < needed {
        return Err(Error::new("io.reverse.disk_full").arg("needed", needed as i64));
    }
    let tmp = folder.join(format!(".reverse-{}.tmp.wav", new_uuid()));
    let result = write(&mut reader, &tmp, info.rate, channels, from, present, lead, cancel, progress)
        .and_then(|()| import_file(&tmp, project, rate, cancel, progress));
    let _ = std::fs::remove_file(&tmp);
    result
}

#[allow(clippy::too_many_arguments)]
fn write(
    reader: &mut Reader,
    tmp: &Path,
    rate: u32,
    channels: usize,
    from: u64,
    frames: u64,
    lead: u64,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<()> {
    let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate, channels };
    let mut writer = Writer::create(tmp, spec, Existing::Fail)?;
    let silence = vec![0.0f32; CHUNK * channels];
    let mut left = lead;
    while left > 0 {
        let n = left.min(CHUNK as u64) as usize;
        writer.write_f32(&silence[..n * channels])?;
        left -= n as u64;
    }
    let mut buf = vec![0.0f32; CHUNK * channels];
    let mut end = from + frames;
    while end > from {
        cancel.check()?;
        let n = (end - from).min(CHUNK as u64) as usize;
        let at = end - n as u64;
        reader.seek(at)?;
        let got = reader.read_f32(&mut buf[..n * channels])?;
        buf[got * channels..n * channels].fill(0.0);
        // The frames of the chunk in the opposite order, each frame's channels kept together
        for i in 0..n / 2 {
            let j = n - 1 - i;
            for c in 0..channels {
                buf.swap(i * channels + c, j * channels + c);
            }
        }
        writer.write_f32(&buf[..n * channels])?;
        end = at;
        progress.set(0.5 * (frames - (end - from)) as f32 / frames as f32);
    }
    writer.sync()?;
    writer.close()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-CLP-042: the reversed stretch has the frames in the opposite order behind the silence
    #[test]
    fn a_stretch_is_written_backwards_behind_the_missing_frames_as_silence() {
        let dir = std::env::temp_dir().join(format!("lm-reverse-{}", new_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.wav");
        let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate: 48_000, channels: 2 };
        let mut w = Writer::create(&src, spec, Existing::Fail).unwrap();
        // 100 frames: the left channel counts up, the right one down
        let data: Vec<f32> = (0..100).flat_map(|i| [i as f32, -(i as f32)]).collect();
        w.write_f32(&data).unwrap();
        w.close().unwrap();
        // 100 asked from frame 10, the source has 90 of them: 10 frames of silence come first
        let done = reverse_range(&src, &dir, 10, 100, 48_000, &Cancel::new(), &Progress::new()).unwrap();
        assert_eq!((done.frames, done.channels), (100, 2));
        let mut r = Reader::open(&done.path).unwrap();
        let mut out = vec![0.0f32; 100 * 2];
        assert_eq!(r.read_f32(&mut out).unwrap(), 100);
        assert!(out[..20].iter().all(|v| *v == 0.0), "the lead is silent");
        assert_eq!((out[20], out[21]), (99.0, -99.0), "the source's last frame comes first");
        assert_eq!((out[198], out[199]), (10.0, -10.0), "and its first asked frame last");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
