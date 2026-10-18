//! SenseVoice-Small (FunASR) speech recognition through ONNX Runtime.
//!
//! Uses the sherpa-onnx export (`model.int8.onnx` + `tokens.txt`), which carries its
//! front-end parameters (LFR window, CMVN statistics, language ids) as model metadata.
//! The front end is reimplemented here to match FunASR's `WavFrontend`:
//! Kaldi fbank (80 mel bins, 25 ms Hamming window, 10 ms shift, no dither) on samples
//! scaled to 16-bit range, then low frame rate stacking (7 frames every 6) and CMVN.
//! Decoding is CTC greedy search; the four leading prompt tokens (language, emotion,
//! event, text normalization) are dropped from the text.

use ndarray::{Array1, Array3};
use ort::execution_providers::CPUExecutionProvider;
use ort::inputs;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use realfft::RealFftPlanner;
use std::f32::consts::PI;
use std::fs;
use std::path::Path;

use super::model::ParakeetError;

const SAMPLE_RATE: f32 = 16000.0;
const FRAME_LENGTH: usize = 400; // 25 ms
const FRAME_SHIFT: usize = 160; // 10 ms
const FFT_SIZE: usize = 512;
const NUM_MEL_BINS: usize = 80;
const PREEMPHASIS: f32 = 0.97;
const LOW_FREQ: f32 = 20.0;
/// SenseVoice is trained on utterances up to about 30 s; longer audio is decoded in pieces
const MAX_CHUNK_SAMPLES: usize = 25 * 16000;

pub struct SenseVoiceModel {
    session: Session,
    tokens: Vec<String>,
    lfr_window_size: usize,
    lfr_window_shift: usize,
    neg_mean: Vec<f32>,
    inv_stddev: Vec<f32>,
    language_auto: i32,
    with_itn: i32,
    blank_id: usize,
    mel_banks: Vec<Vec<(usize, f32)>>,
    window: Vec<f32>,
}

fn invalid_data(message: String) -> ParakeetError {
    ParakeetError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, message))
}

fn parse_floats(text: &str) -> Vec<f32> {
    text.split(',').filter_map(|v| v.trim().parse::<f32>().ok()).collect()
}

fn mel_scale(freq: f32) -> f32 {
    1127.0 * (1.0 + freq / 700.0).ln()
}

/// Triangular mel filters as Kaldi builds them, over FFT bins 0..FFT_SIZE/2 (Nyquist bin left out)
fn mel_banks() -> Vec<Vec<(usize, f32)>> {
    let bin_width = SAMPLE_RATE / FFT_SIZE as f32;
    let mel_low = mel_scale(LOW_FREQ);
    let mel_high = mel_scale(SAMPLE_RATE / 2.0);
    let delta = (mel_high - mel_low) / (NUM_MEL_BINS as f32 + 1.0);
    (0..NUM_MEL_BINS)
        .map(|bin| {
            let left = mel_low + bin as f32 * delta;
            let center = left + delta;
            let right = center + delta;
            (0..FFT_SIZE / 2)
                .filter_map(|k| {
                    let mel = mel_scale(bin_width * k as f32);
                    let weight = ((mel - left) / (center - left)).min((right - mel) / (right - center));
                    (weight > 0.0).then_some((k, weight))
                })
                .collect()
        })
        .collect()
}

impl SenseVoiceModel {
    pub fn new<P: AsRef<Path>>(model_dir: P) -> Result<Self, ParakeetError> {
        crate::ensure_onnx_runtime_available().map_err(|error| ParakeetError::RuntimeUnavailable(error.to_string()))?;
        let model_dir = model_dir.as_ref();
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_execution_providers(vec![CPUExecutionProvider::default().build()])?
            .commit_from_file(model_dir.join("model.int8.onnx"))?;

        let meta = session.metadata()?;
        let get = |key: &str| -> Result<String, ParakeetError> {
            meta.custom(key)?.ok_or_else(|| invalid_data(format!("SenseVoice model metadata is missing {}", key)))
        };
        let get_int = |key: &str| -> Result<i64, ParakeetError> {
            get(key)?.trim().parse::<i64>().map_err(|_| invalid_data(format!("SenseVoice metadata {} is not a number", key)))
        };
        let lfr_window_size = get_int("lfr_window_size")? as usize;
        let lfr_window_shift = get_int("lfr_window_shift")? as usize;
        let neg_mean = parse_floats(&get("neg_mean")?);
        let inv_stddev = parse_floats(&get("inv_stddev")?);
        let feature_dim = NUM_MEL_BINS * lfr_window_size;
        if neg_mean.len() != feature_dim || inv_stddev.len() != feature_dim {
            return Err(invalid_data(format!(
                "SenseVoice CMVN has {} / {} values, expected {}",
                neg_mean.len(),
                inv_stddev.len(),
                feature_dim
            )));
        }
        let language_auto = get_int("lang_auto")? as i32;
        let with_itn = get_int("with_itn")? as i32;
        drop(meta);

        let tokens = Self::load_tokens(model_dir)?;
        log::info!("Loaded SenseVoice model with {} tokens", tokens.len());

        let window = (0..FRAME_LENGTH)
            .map(|n| 0.54 - 0.46 * (2.0 * PI * n as f32 / (FRAME_LENGTH as f32 - 1.0)).cos())
            .collect();

        Ok(Self {
            session,
            tokens,
            lfr_window_size,
            lfr_window_shift,
            neg_mean,
            inv_stddev,
            language_auto,
            with_itn,
            blank_id: 0,
            mel_banks: mel_banks(),
            window,
        })
    }

    /// tokens.txt: one "token id" per line
    fn load_tokens(model_dir: &Path) -> Result<Vec<String>, ParakeetError> {
        let content = fs::read_to_string(model_dir.join("tokens.txt"))?;
        let mut pairs = Vec::new();
        for line in content.lines() {
            if let Some((token, id)) = line.trim_end().rsplit_once(' ') {
                if let Ok(id) = id.parse::<usize>() {
                    pairs.push((token.to_string(), id));
                }
            }
        }
        let size = pairs.iter().map(|(_, id)| id + 1).max().unwrap_or(0);
        let mut tokens = vec![String::new(); size];
        for (token, id) in pairs {
            tokens[id] = token;
        }
        if tokens.is_empty() {
            return Err(invalid_data("SenseVoice tokens.txt is empty".to_string()));
        }
        Ok(tokens)
    }

    /// Log mel filterbank energies, one row of NUM_MEL_BINS per 10 ms frame
    pub fn fbank(&self, samples: &[f32]) -> Vec<Vec<f32>> {
        if samples.len() < FRAME_LENGTH {
            return Vec::new();
        }
        let num_frames = 1 + (samples.len() - FRAME_LENGTH) / FRAME_SHIFT;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let mut input = fft.make_input_vec();
        let mut spectrum = fft.make_output_vec();
        let mut frames = Vec::with_capacity(num_frames);

        for f in 0..num_frames {
            let start = f * FRAME_SHIFT;
            // FunASR feeds 16-bit scaled samples to Kaldi fbank
            let mut frame: Vec<f32> = samples[start..start + FRAME_LENGTH].iter().map(|s| s * 32768.0).collect();
            let mean = frame.iter().sum::<f32>() / FRAME_LENGTH as f32;
            frame.iter_mut().for_each(|s| *s -= mean);
            for i in (1..FRAME_LENGTH).rev() {
                frame[i] -= PREEMPHASIS * frame[i - 1];
            }
            frame[0] -= PREEMPHASIS * frame[0];

            input.iter_mut().for_each(|v| *v = 0.0);
            for (i, (s, w)) in frame.iter().zip(&self.window).enumerate() {
                input[i] = s * w;
            }
            // Only fails on buffer length mismatch, which the planner's own buffers rule out
            let _ = fft.process(&mut input, &mut spectrum);
            let power: Vec<f32> = spectrum.iter().map(|c| c.norm_sqr()).collect();

            let mel: Vec<f32> = self
                .mel_banks
                .iter()
                .map(|bank| {
                    let energy: f32 = bank.iter().map(|(k, w)| power[*k] * w).sum();
                    energy.max(f32::EPSILON).ln()
                })
                .collect();
            frames.push(mel);
        }
        frames
    }

    /// Stack lfr_window_size frames every lfr_window_shift frames (left padded by repeating
    /// the first frame), then apply CMVN
    pub fn lfr_cmvn(&self, fbank: &[Vec<f32>]) -> Array3<f32> {
        let (m, n) = (self.lfr_window_size, self.lfr_window_shift);
        let dim = NUM_MEL_BINS * m;
        if fbank.is_empty() {
            return Array3::zeros((1, 0, dim));
        }
        let left_pad = (m - 1) / 2;
        let padded: Vec<&Vec<f32>> = std::iter::repeat(&fbank[0]).take(left_pad).chain(fbank.iter()).collect();
        let out_frames = fbank.len().div_ceil(n);
        let mut features = Array3::<f32>::zeros((1, out_frames, dim));
        for t in 0..out_frames {
            for j in 0..m {
                // Past the end, the last frame repeats
                let frame = padded.get(t * n + j).copied().unwrap_or(padded[padded.len() - 1]);
                for (d, value) in frame.iter().enumerate() {
                    let index = j * NUM_MEL_BINS + d;
                    features[[0, t, index]] = (value + self.neg_mean[index]) * self.inv_stddev[index];
                }
            }
        }
        features
    }

    fn decode_chunk(&mut self, samples: &[f32]) -> Result<String, ParakeetError> {
        let features = self.lfr_cmvn(&self.fbank(samples));
        let frames = features.shape()[1];
        if frames == 0 {
            return Ok(String::new());
        }
        let length = Array1::from_vec(vec![frames as i32]);
        let language = Array1::from_vec(vec![self.language_auto]);
        let text_norm = Array1::from_vec(vec![self.with_itn]);
        let blank_id = self.blank_id;
        let ids = {
        let outputs = self.session.run(inputs![
            "x" => TensorRef::from_array_view(features.view())?,
            "x_length" => TensorRef::from_array_view(length.view())?,
            "language" => TensorRef::from_array_view(language.view())?,
            "text_norm" => TensorRef::from_array_view(text_norm.view())?,
        ])?;
        let logits = outputs
            .get("logits")
            .ok_or_else(|| ParakeetError::OutputNotFound("logits".to_string()))?
            .try_extract_array::<f32>()?;
        let shape = logits.shape().to_vec();
        if shape.len() != 3 {
            return Err(ParakeetError::TensorShape("logits".to_string()));
        }

        let mut ids = Vec::new();
        let mut previous = None;
        for t in 0..shape[1] {
            let mut best = (0usize, f32::NEG_INFINITY);
            for v in 0..shape[2] {
                let score = logits[[0, t, v]];
                if score > best.1 {
                    best = (v, score);
                }
            }
            if Some(best.0) != previous && best.0 != blank_id {
                ids.push(best.0);
            }
            previous = Some(best.0);
        }
        ids
        };
        Ok(self.ids_to_text(&ids))
    }

    fn ids_to_text(&self, ids: &[usize]) -> String {
        let text: String = ids
            .iter()
            .filter_map(|id| self.tokens.get(*id))
            // Prompt tokens such as <|zh|><|NEUTRAL|><|Speech|><|withitn|>
            .filter(|token| !(token.starts_with("<|") && token.ends_with("|>")))
            .map(|token| token.replace('\u{2581}', " "))
            .collect();
        text.trim().to_string()
    }

    pub fn transcribe_samples(&mut self, samples: &[f32]) -> Result<String, ParakeetError> {
        let mut pieces = Vec::new();
        for chunk in samples.chunks(MAX_CHUNK_SAMPLES) {
            let text = self.decode_chunk(chunk)?;
            if !text.is_empty() {
                pieces.push(text);
            }
        }
        Ok(join_pieces(&pieces))
    }
}

/// Chinese pieces join directly; a space only goes between two Latin words
fn join_pieces(pieces: &[String]) -> String {
    let mut out = String::new();
    for piece in pieces {
        let needs_space = out.chars().last().is_some_and(|c| c.is_ascii_alphanumeric())
            && piece.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
        if needs_space {
            out.push(' ');
        }
        out.push_str(piece);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mel_banks_cover_the_spectrum_in_order() {
        let banks = mel_banks();
        assert_eq!(banks.len(), NUM_MEL_BINS);
        // 20 Hz is bin 0.64, so the first filter starts at bin 1
        assert_eq!(banks[0].first().map(|(k, _)| *k), Some(1));
        assert!(banks.windows(2).all(|w| w[0][0].0 <= w[1][0].0));
        assert!(banks.iter().all(|b| b.iter().all(|(_, w)| *w > 0.0 && *w <= 1.0)));
    }

    #[test]
    fn pieces_join_without_spaces_in_chinese() {
        assert_eq!(join_pieces(&["开会了".to_string(), "大家好".to_string()]), "开会了大家好");
        assert_eq!(join_pieces(&["use the API".to_string(), "now".to_string()]), "use the API now");
    }
}
