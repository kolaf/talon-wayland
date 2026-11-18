use crate::speech::engine::SpeechError;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SizedSample, Stream};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const TARGET_SAMPLE_RATE: u32 = 16_000;
const VAD_FRAME_SAMPLES: usize = 512;
const VAD_FRAME_MS: u32 = 32;

/// Energy-based voice activity detection and utterance segmentation parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct VadConfig {
    pub speech_rms_threshold: f32,
    pub silence_rms_threshold: f32,
    pub min_speech_ms: u32,
    pub min_silence_ms: u32,
    pub max_speech_ms: u32,
    pub pre_roll_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            speech_rms_threshold: 0.015,
            silence_rms_threshold: 0.008,
            min_speech_ms: 180,
            min_silence_ms: 450,
            max_speech_ms: 8_000,
            pre_roll_ms: 240,
        }
    }
}

/// Averages interleaved multi-channel PCM samples into mono `f32`.
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let inv = 1.0 / channels as f32;
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() * inv)
        .collect()
}

/// Linearly resamples mono `f32` PCM from `src_rate` Hz to `16_000` Hz.
pub fn resample_linear_16khz(mono: &[f32], src_rate: u32) -> Vec<f32> {
    if mono.is_empty() || src_rate == 0 {
        return Vec::new();
    }
    if src_rate == TARGET_SAMPLE_RATE {
        return mono.to_vec();
    }

    let out_len = ((mono.len() as f64) * (TARGET_SAMPLE_RATE as f64) / (src_rate as f64))
        .round()
        .max(1.0) as usize;
    let ratio = (src_rate as f64) / (TARGET_SAMPLE_RATE as f64);
    let max_idx = mono.len() - 1;

    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = (i as f64) * ratio;
        let idx0 = (pos.floor() as usize).min(max_idx);
        let idx1 = (idx0 + 1).min(max_idx);
        let frac = (pos - idx0 as f64) as f32;
        let sample = mono[idx0] * (1.0 - frac) + mono[idx1] * frac;
        out.push(sample);
    }
    out
}

/// Computes root-mean-square energy for a slice of PCM samples.
pub fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Stateful streaming segmenter that splits 16 kHz mono audio into spoken command utterances.
#[derive(Debug)]
pub struct UtteranceSegmenter {
    config: VadConfig,
    pre_roll_samples: usize,
    pending: Vec<f32>,
    pre_roll: VecDeque<f32>,
    speech_buffer: Vec<f32>,
    completed: VecDeque<Vec<f32>>,
    in_speech: bool,
    speech_ms: u32,
    silence_ms: u32,
}

impl UtteranceSegmenter {
    pub fn new(config: VadConfig) -> Self {
        let pre_roll_samples =
            ((config.pre_roll_ms as usize) * (TARGET_SAMPLE_RATE as usize)) / 1000;
        Self {
            config,
            pre_roll_samples,
            pending: Vec::new(),
            pre_roll: VecDeque::with_capacity(pre_roll_samples + VAD_FRAME_SAMPLES),
            speech_buffer: Vec::new(),
            completed: VecDeque::new(),
            in_speech: false,
            speech_ms: 0,
            silence_ms: 0,
        }
    }

    /// Feeds 16 kHz mono samples and returns a completed utterance when end-of-speech is detected.
    pub fn push_samples(&mut self, samples_16khz: &[f32]) -> Option<Vec<f32>> {
        self.pending.extend_from_slice(samples_16khz);

        let mut offset = 0;
        let mut frame = [0.0f32; VAD_FRAME_SAMPLES];
        while offset + VAD_FRAME_SAMPLES <= self.pending.len() {
            frame.copy_from_slice(&self.pending[offset..offset + VAD_FRAME_SAMPLES]);
            self.process_frame(&frame);
            offset += VAD_FRAME_SAMPLES;
        }
        if offset > 0 {
            self.pending.drain(..offset);
        }

        self.completed.pop_front()
    }

    /// Flushes any in-progress utterance (for finite streams or timeouts).
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if let Some(done) = self.completed.pop_front() {
            return Some(done);
        }
        if self.in_speech {
            if !self.pending.is_empty() {
                self.speech_buffer.append(&mut self.pending);
            }
            let buf = std::mem::take(&mut self.speech_buffer);
            let valid = self.speech_ms >= self.config.min_speech_ms;
            self.reset_speech_state();
            if valid {
                return Some(buf);
            }
        }
        None
    }

    fn process_frame(&mut self, frame: &[f32]) {
        let rms = frame_rms(frame);
        if !self.in_speech {
            if rms >= self.config.speech_rms_threshold {
                self.in_speech = true;
                self.speech_buffer.extend(self.pre_roll.drain(..));
                self.speech_buffer.extend_from_slice(frame);
                self.speech_ms = VAD_FRAME_MS;
                self.silence_ms = 0;
            } else {
                for &s in frame {
                    if self.pre_roll.len() >= self.pre_roll_samples && self.pre_roll_samples > 0 {
                        self.pre_roll.pop_front();
                    }
                    if self.pre_roll_samples > 0 {
                        self.pre_roll.push_back(s);
                    }
                }
            }
            return;
        }

        self.speech_buffer.extend_from_slice(frame);
        if rms < self.config.silence_rms_threshold {
            self.silence_ms += VAD_FRAME_MS;
        } else {
            self.speech_ms += VAD_FRAME_MS;
            self.silence_ms = 0;
        }

        let total_ms = (self.speech_buffer.len() as u64 * 1000) / (TARGET_SAMPLE_RATE as u64);
        let max_reached = total_ms >= self.config.max_speech_ms as u64;
        let silence_ended = self.silence_ms >= self.config.min_silence_ms;

        if silence_ended || max_reached {
            let buf = std::mem::take(&mut self.speech_buffer);
            let valid = self.speech_ms >= self.config.min_speech_ms;
            self.reset_speech_state();
            if valid {
                self.completed.push_back(buf);
            }
        }
    }

    fn reset_speech_state(&mut self) {
        self.in_speech = false;
        self.speech_ms = 0;
        self.silence_ms = 0;
    }
}

/// Loads a `.wav` file (int or float PCM, arbitrary channels/rate) as 16 kHz mono `f32` PCM.
pub fn load_wav_16khz_mono(path: &Path) -> Result<Vec<f32>, SpeechError> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| SpeechError::RecognitionFailed(format!("{}: {e}", path.display())))?;
    let spec = reader.spec();

    let raw_f32: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<f32>, _>>()
            .map_err(|e| SpeechError::RecognitionFailed(format!("{}: {e}", path.display())))?,
        hound::SampleFormat::Int => {
            let max_amp = ((1_i64 << (spec.bits_per_sample.saturating_sub(1))) as f32).max(1.0);
            reader
                .samples::<i32>()
                .map(|r| r.map(|v| (v as f32) / max_amp))
                .collect::<Result<Vec<f32>, _>>()
                .map_err(|e| SpeechError::RecognitionFailed(format!("{}: {e}", path.display())))?
        }
    };

    let mono = downmix_to_mono(&raw_f32, spec.channels as usize);
    Ok(resample_linear_16khz(&mono, spec.sample_rate))
}

/// Metadata describing an available audio input device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    pub index: usize,
    pub name: String,
    pub is_default: bool,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
}

/// Enumerates available microphone / audio input devices with metadata.
pub fn enumerate_input_devices() -> Result<Vec<AudioDeviceInfo>, SpeechError> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.description().ok())
        .map(|desc| desc.name().to_string());

    let devices = host
        .input_devices()
        .map_err(|e| SpeechError::Unavailable(format!("failed to list input devices: {e}")))?;

    let mut infos = Vec::new();
    for (i, dev) in devices.enumerate() {
        let name = dev
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| format!("input-device-{}", i + 1));
        let cfg = dev.default_input_config().ok();
        let is_default = default_name
            .as_deref()
            .is_some_and(|def| def == name);
        infos.push(AudioDeviceInfo {
            index: i + 1,
            name,
            is_default,
            sample_rate: cfg.as_ref().map(|c| c.sample_rate()),
            channels: cfg.as_ref().map(|c| c.channels()),
        });
    }
    Ok(infos)
}

/// Enumerates available microphone / audio input device names on this host.
pub fn list_input_devices() -> Result<Vec<String>, SpeechError> {
    Ok(enumerate_input_devices()?
        .into_iter()
        .map(|d| d.name)
        .collect())
}

/// Resolves a user device selector (1-based index, exact name, or case-insensitive substring).
pub fn resolve_input_device_selector(
    selector: &str,
    devices: &[AudioDeviceInfo],
) -> Result<AudioDeviceInfo, SpeechError> {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return Err(SpeechError::Unavailable(
            "empty audio device selector".into(),
        ));
    }

    if let Ok(idx) = trimmed.parse::<usize>() {
        return devices
            .iter()
            .find(|d| d.index == idx)
            .cloned()
            .ok_or_else(|| {
                SpeechError::Unavailable(format!(
                    "audio input device index {idx} out of range (1..={})",
                    devices.len()
                ))
            });
    }

    if let Some(exact) = devices
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(trimmed))
    {
        return Ok(exact.clone());
    }

    let needle = trimmed.to_ascii_lowercase();
    if let Some(substr) = devices
        .iter()
        .find(|d| d.name.to_ascii_lowercase().contains(&needle))
    {
        return Ok(substr.clone());
    }

    Err(SpeechError::Unavailable(format!(
        "no audio input device matching `{trimmed}`"
    )))
}

/// Live microphone capture stream feeding 16 kHz mono utterances via `cpal`.
pub struct MicrophoneCapture {
    _stream: Stream,
    rx: Receiver<Vec<f32>>,
    segmenter: UtteranceSegmenter,
    device_name: String,
}

impl std::fmt::Debug for MicrophoneCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicrophoneCapture")
            .field("device_name", &self.device_name)
            .finish_non_exhaustive()
    }
}

impl MicrophoneCapture {
    pub fn open(device_selector: Option<&str>, vad_config: VadConfig) -> Result<Self, SpeechError> {
        let host = cpal::default_host();
        let device = match device_selector {
            Some(target) => {
                let infos = enumerate_input_devices()?;
                let resolved = resolve_input_device_selector(target, &infos)?;
                let mut devices = host.input_devices().map_err(|e| {
                    SpeechError::Unavailable(format!("failed to enumerate input devices: {e}"))
                })?;
                devices.nth(resolved.index - 1).ok_or_else(|| {
                    SpeechError::Unavailable(format!("input device `{}` disappeared", resolved.name))
                })?
            }
            None => host.default_input_device().ok_or_else(|| {
                SpeechError::Unavailable("no default audio input device found".into())
            })?,
        };

        let resolved_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "default".into());

        let supported_cfg = device.default_input_config().map_err(|e| {
            SpeechError::Unavailable(format!("failed to query input config for `{resolved_name}`: {e}"))
        })?;

        let sample_format = supported_cfg.sample_format();
        let channels = supported_cfg.channels() as usize;
        let sample_rate = supported_cfg.sample_rate();
        let stream_cfg: cpal::StreamConfig = supported_cfg.into();

        let (tx, rx) = mpsc::channel::<Vec<f32>>();

        let stream = match sample_format {
            SampleFormat::F32 => {
                build_typed_input_stream::<f32>(&device, &stream_cfg, channels, sample_rate, tx)?
            }
            SampleFormat::I16 => {
                build_typed_input_stream::<i16>(&device, &stream_cfg, channels, sample_rate, tx)?
            }
            SampleFormat::U16 => {
                build_typed_input_stream::<u16>(&device, &stream_cfg, channels, sample_rate, tx)?
            }
            SampleFormat::I32 => {
                build_typed_input_stream::<i32>(&device, &stream_cfg, channels, sample_rate, tx)?
            }
            SampleFormat::I8 => {
                build_typed_input_stream::<i8>(&device, &stream_cfg, channels, sample_rate, tx)?
            }
            SampleFormat::U8 => {
                build_typed_input_stream::<u8>(&device, &stream_cfg, channels, sample_rate, tx)?
            }
            other => {
                return Err(SpeechError::Unavailable(format!(
                    "unsupported microphone sample format: {other:?}"
                )));
            }
        };

        stream.play().map_err(|e| {
            SpeechError::Unavailable(format!("failed to start audio input stream: {e}"))
        })?;

        Ok(Self {
            _stream: stream,
            rx,
            segmenter: UtteranceSegmenter::new(vad_config),
            device_name: resolved_name,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Blocks until the next speech utterance is segmented (or `timeout` expires).
    pub fn capture_next_utterance(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<Vec<f32>, SpeechError> {
        let deadline = timeout.map(|t| Instant::now() + t);

        loop {
            let chunk = match deadline {
                Some(dl) => {
                    let remaining = dl.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        if let Some(flushed) = self.segmenter.flush() {
                            return Ok(flushed);
                        }
                        return Err(SpeechError::RecognitionFailed(
                            "timed out waiting for speech utterance".into(),
                        ));
                    }
                    match self.rx.recv_timeout(remaining) {
                        Ok(c) => c,
                        Err(RecvTimeoutError::Timeout) => {
                            if let Some(flushed) = self.segmenter.flush() {
                                return Ok(flushed);
                            }
                            return Err(SpeechError::RecognitionFailed(
                                "timed out waiting for speech utterance".into(),
                            ));
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            return Err(SpeechError::Unavailable(
                                "audio input stream disconnected".into(),
                            ));
                        }
                    }
                }
                None => self.rx.recv().map_err(|_| {
                    SpeechError::Unavailable("audio input stream disconnected".into())
                })?,
            };

            if let Some(utterance) = self.segmenter.push_samples(&chunk) {
                return Ok(utterance);
            }
        }
    }

    /// Waits up to `wait` for incoming audio and returns `Ok(Some(utterance))` once a
    /// complete spoken utterance ends, or `Ok(None)` if silence/mid-utterance continues.
    pub fn poll_next_utterance(
        &mut self,
        wait: Duration,
    ) -> Result<Option<Vec<f32>>, SpeechError> {
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            match self.rx.recv_timeout(remaining) {
                Ok(chunk) => {
                    if let Some(utterance) = self.segmenter.push_samples(&chunk) {
                        return Ok(Some(utterance));
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(SpeechError::Unavailable(
                        "audio input stream disconnected".into(),
                    ));
                }
            }
        }
    }
}

fn build_typed_input_stream<T>(
    device: &cpal::Device,
    stream_cfg: &cpal::StreamConfig,
    channels: usize,
    sample_rate: u32,
    tx: mpsc::Sender<Vec<f32>>,
) -> Result<Stream, SpeechError>
where
    T: SizedSample + Sample,
    f32: cpal::FromSample<T>,
{
    device
        .build_input_stream(
            stream_cfg.clone(),
            move |data: &[T], _| {
                let f32_samples: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
                let mono = downmix_to_mono(&f32_samples, channels);
                let resampled = resample_linear_16khz(&mono, sample_rate);
                if !resampled.is_empty() {
                    let _ = tx.send(resampled);
                }
            },
            |err| {
                eprintln!("audio input stream error: {err}");
            },
            None,
        )
        .map_err(|e| SpeechError::Unavailable(format!("failed to build input stream: {e}")))
}
