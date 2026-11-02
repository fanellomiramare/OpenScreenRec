//! Temporary WAV recording: audio chunks land on disk as 16-bit PCM
//! while the video encodes, then the mux step combines them. (M6
//! replaces this with the pure-Rust muxer writing compressed audio
//! directly.)

use std::io::BufWriter;
use std::path::Path;

use crate::{AudioChunk, AudioConfig, AudioError};

pub struct WavAudioWriter {
    writer: hound::WavWriter<BufWriter<std::fs::File>>,
}

impl WavAudioWriter {
    pub fn create(path: &Path, config: AudioConfig) -> Result<Self, AudioError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let spec = hound::WavSpec {
            channels: config.channels,
            sample_rate: config.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        Ok(Self {
            writer: hound::WavWriter::create(path, spec)?,
        })
    }

    pub fn write(&mut self, chunk: &AudioChunk) -> Result<(), AudioError> {
        for sample in &chunk.samples {
            let quantized = (sample.clamp(-1.0, 1.0) * 32_767.0) as i16;
            self.writer.write_sample(quantized)?;
        }
        Ok(())
    }

    /// Rewrite the header with the final sizes; the file is only valid
    /// after this.
    pub fn finalize(self) -> Result<(), AudioError> {
        self.writer.finalize()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceKind;

    #[test]
    fn wav_round_trip_preserves_samples() {
        let dir = std::env::temp_dir().join(format!("osr-audio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.wav");

        let chunk = AudioChunk {
            kind: SourceKind::Microphone,
            pts_us: 0,
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.0, 0.5, -0.5, 1.0, -1.0, 0.25],
        };

        {
            let mut writer = WavAudioWriter::create(
                &path,
                AudioConfig {
                    sample_rate: chunk.sample_rate,
                    channels: chunk.channels,
                },
            )
            .unwrap();
            writer.write(&chunk).unwrap();
            writer.finalize().unwrap();
        }

        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().channels, 2);
        let round_tripped: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        let expected = [0, 16383, -16383, 32767, -32767, 8191];
        assert_eq!(round_tripped, expected);

        std::fs::remove_dir_all(dir).ok();
    }
}
