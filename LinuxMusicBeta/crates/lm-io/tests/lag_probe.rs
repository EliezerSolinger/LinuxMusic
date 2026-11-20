mod common;
use common::TempDir;
use lm_base::{Cancel, Progress};
use lm_io::stretch::{StretchJob, render_stretch};
use lm_sys_sndfile::{Existing, Reader, Sample, WriteFormat, WriteSpec, Writer};
#[test]
fn probe() {
    let dir = TempDir::new("lag");
    let path = dir.0.join("take.wav");
    let spec = WriteSpec { format: WriteFormat::Wav(Sample::Float32), rate: 48_000, channels: 1 };
    let mut w = Writer::create(&path, spec, Existing::Fail).unwrap();
    let frames: Vec<f32> = (0..192_000).map(|n| if n % 4_800 == 2_400 { 1.0 } else { 0.0 }).collect();
    w.write_f32(&frames).unwrap();
    w.close().unwrap();
    let plans: Vec<Vec<(u64, u64)>> = vec![
        vec![(0, 0), (96_000, 48_000), (192_000, 192_000)],
        vec![(0, 0), (96_000, 144_000), (192_000, 192_000)],
        vec![(0, 0), (96_000, 96_000), (192_000, 288_000)],
        vec![(0, 0), (48_000, 60_000), (96_000, 100_000), (144_000, 130_000), (192_000, 200_000)],
    ];
    for (i, kf) in plans.into_iter().enumerate() {
        let key = format!("{:064x}", i + 1);
        let out = *kf.last().unwrap();
        let job = StretchJob { source: path.clone(), project: dir.0.clone(), key: key.clone(), key_frames: kf.clone(), output_frames: out.1, rate: 48_000 };
        let p = render_stretch(&job, &Cancel::new(), &Progress::new()).unwrap();
        let mut r = Reader::open(&p).unwrap();
        let n = r.info().frames as usize;
        let mut buf = vec![0.0f32; n];
        r.read_f32(&mut buf).unwrap();
        let map = |x: u64| { let i = kf.windows(2).position(|w| x < w[1].0).unwrap(); let (a,b)=(kf[i],kf[i+1]); a.1 as f64 + (x-a.0) as f64 * (b.1-a.1) as f64/(b.0-a.0) as f64 };
        let mut errs = Vec::new();
        for c in (2_400..190_000).step_by(4_800) {
            let at = map(c as u64) as usize;
            let lo = at.saturating_sub(2_000); let hi = (at + 2_000).min(n);
            let found = (lo..hi).max_by(|a, b| buf[*a].abs().total_cmp(&buf[*b].abs())).unwrap();
            errs.push(found as i64 - at as i64);
        }
        eprintln!("plan {i}: max err {} errs {:?}", errs.iter().map(|e| e.abs()).max().unwrap(), errs);
        std::fs::remove_file(p).unwrap();
    }
}
