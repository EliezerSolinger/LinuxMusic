//! The sample the slicer reads: its frames as stereo (a mono sample on both sides; past two channels, the first two),
//! and its rate.

/// The sample: its frames as stereo (a mono sample on both sides; past two channels, the first two), and its rate
#[derive(Debug)]
pub struct Audio {
    pub frames: Vec<[f32; 2]>,
    pub rate: f32,
}

impl Audio {
    /// The resource's audio as the player reads it
    pub fn of(data: &lm_render::SampleData) -> Self {
        let left = data.channel(0);
        let right = if data.channels() > 1 { data.channel(1) } else { left };
        Self { frames: left.iter().zip(right).map(|(l, r)| [*l, *r]).collect(), rate: data.rate() as f32 }
    }
}
