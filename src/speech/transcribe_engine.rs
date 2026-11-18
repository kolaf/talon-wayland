use crate::config::load_selected_model;
use crate::speech::audio::{MicrophoneCapture, VadConfig};
use crate::speech::candidate::SpeechCandidate;
use crate::speech::engine::{SpeechEngine, SpeechError};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use transcribe_cpp::{
    Feature, Model, Pnc, RunExtension, RunOptions, Session, WhisperRunOptions,
};

const MAX_BIAS_PHRASES: usize = 128;

/// Searches `VOICE_RUNTIME_MODEL`, persisted model selection, Hugging Face Hub cache
/// (`$HF_HOME/hub`, `~/.cache/huggingface/hub`), Handy cache/data directories, and `./models`
/// for local `.gguf` and `ggml-*.bin` speech models (prioritizing Conformer/Parakeet models).
pub fn discover_available_models() -> Vec<PathBuf> {
    let mut prioritized = Vec::new();
    let mut seen = HashSet::new();

    if let Ok(env_path) = std::env::var("VOICE_RUNTIME_MODEL") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.is_file() && seen.insert(p.clone()) {
                prioritized.push(p);
            }
        }
    }

    if let Some(saved) = load_selected_model(None) {
        if saved.is_file() && seen.insert(saved.clone()) {
            prioritized.push(saved);
        }
    }

    let roots = default_model_search_roots();
    for model in discover_models_in_roots(&roots) {
        if seen.insert(model.clone()) {
            prioritized.push(model);
        }
    }

    prioritized
}

/// Returns the highest-priority discovered local speech model, if any.
pub fn discover_default_model_path() -> Option<PathBuf> {
    discover_available_models().into_iter().next()
}

/// Scans `roots` recursively (up to depth 5) for `.gguf` and `ggml-*.bin` speech models,
/// excluding VAD/Silero models and sorting Conformer/Parakeet models first.
pub fn discover_models_in_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        if root.exists() {
            collect_model_files(root, 0, 5, &mut found);
        }
    }

    found.sort_by(|a, b| {
        model_priority_rank(a)
            .cmp(&model_priority_rank(b))
            .then_with(|| a.cmp(b))
    });

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for path in found {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

fn default_model_search_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(hf_home) = std::env::var("HF_HOME") {
        let trimmed = hf_home.trim();
        if !trimmed.is_empty() {
            roots.push(PathBuf::from(trimmed).join("hub"));
        }
    }
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        let base = PathBuf::from(xdg_cache);
        roots.push(base.join("huggingface/hub"));
        roots.push(base.join("com.pais.handy"));
        roots.push(base.join("handy"));
        roots.push(base.join("voice-runtime"));
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        let base = PathBuf::from(xdg_data);
        roots.push(base.join("com.pais.handy"));
        roots.push(base.join("handy"));
        roots.push(base.join("voice-runtime"));
    }
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        let base = PathBuf::from(xdg_config);
        roots.push(base.join("com.pais.handy"));
    }
    if let Ok(home) = std::env::var("HOME") {
        let home_dir = PathBuf::from(home);
        let cache = home_dir.join(".cache");
        roots.push(cache.join("huggingface/hub"));
        roots.push(cache.join("com.pais.handy"));
        roots.push(cache.join("handy"));
        roots.push(cache.join("voice-runtime"));

        let data = home_dir.join(".local/share");
        roots.push(data.join("com.pais.handy"));
        roots.push(data.join("handy"));
        roots.push(data.join("voice-runtime"));

        roots.push(home_dir.join(".config/com.pais.handy"));
    }
    roots.push(PathBuf::from("models"));
    roots
}

fn model_priority_rank(path: &Path) -> u8 {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.contains("parakeet") || lower.contains("conformer") {
        0
    } else if lower.contains("moonshine") || lower.contains("canary") {
        1
    } else if lower.ends_with(".gguf") {
        2
    } else {
        3
    }
}

fn collect_model_files(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if matches!(dir_name, "blobs" | "refs" | ".git") {
                continue;
            }
            collect_model_files(&path, depth + 1, max_depth, out);
        } else if path.is_file() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if name.contains("silero") || name.contains("vad") {
                continue;
            }
            let is_gguf = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"));
            let is_whisper_bin = name.starts_with("ggml-") && name.ends_with(".bin");
            if is_gguf || is_whisper_bin {
                out.push(path);
            }
        }
    }
}

/// Command-oriented speech recognizer backed by `transcribe-cpp` and live `cpal` capture.
pub struct TranscribeCppEngine {
    model: Model,
    session: Session,
    mic: Option<MicrophoneCapture>,
    pending_pcm: VecDeque<Vec<f32>>,
    model_path: PathBuf,
}

impl std::fmt::Debug for TranscribeCppEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscribeCppEngine")
            .field("model_path", &self.model_path)
            .field("arch", &self.model.arch())
            .field("has_mic", &self.mic.is_some())
            .field("queued_buffers", &self.pending_pcm.len())
            .finish()
    }
}

impl TranscribeCppEngine {
    pub fn load(model_path: &Path) -> Result<Self, SpeechError> {
        let model = Model::load(model_path).map_err(|e| {
            SpeechError::Unavailable(format!("failed to load model `{}`: {e}", model_path.display()))
        })?;
        let session = model.session().map_err(|e| {
            SpeechError::Unavailable(format!(
                "failed to create transcribe session for `{}`: {e}",
                model_path.display()
            ))
        })?;

        Ok(Self {
            model,
            session,
            mic: None,
            pending_pcm: VecDeque::new(),
            model_path: model_path.to_path_buf(),
        })
    }

    pub fn with_microphone(
        &mut self,
        device_name: Option<&str>,
        vad_config: VadConfig,
    ) -> Result<(), SpeechError> {
        let capture = MicrophoneCapture::open(device_name, vad_config)?;
        self.mic = Some(capture);
        Ok(())
    }

    pub fn microphone_device_name(&self) -> Option<&str> {
        self.mic.as_ref().map(|m| m.device_name())
    }

    pub fn model_arch(&self) -> String {
        self.model.arch()
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    pub fn enqueue_pcm_16khz(&mut self, pcm: Vec<f32>) {
        self.pending_pcm.push_back(pcm);
    }

    pub fn transcribe_pcm(
        &mut self,
        pcm_16khz: &[f32],
        phrases: &[String],
    ) -> Result<Vec<SpeechCandidate>, SpeechError> {
        if pcm_16khz.is_empty() {
            return Ok(Vec::new());
        }

        let mut options = RunOptions::default();
        options.pnc = Pnc::Off;
        options.language = Some("en".to_string());

        let bias_phrases = collect_bias_phrases(phrases);
        if !bias_phrases.is_empty() {
            if self.model.supports(Feature::Vocabulary) {
                options.vocabulary = bias_phrases;
            } else if self.model.supports(Feature::ContextPrompt) {
                options.prompt = Some(bias_phrases.join(", "));
            } else if self.model.supports(Feature::InitialPrompt) {
                options.family = Some(RunExtension::Whisper(WhisperRunOptions {
                    initial_prompt: Some(bias_phrases.join(", ")),
                    condition_on_prev_tokens: Some(false),
                    temperature: Some(0.0),
                    ..Default::default()
                }));
            }
        }

        let transcript = self.session.run(pcm_16khz, &options).map_err(|e| {
            SpeechError::RecognitionFailed(format!("transcribe-cpp inference failed: {e}"))
        })?;

        Ok(rank_transcript_candidates(&transcript.text, phrases))
    }

    /// Polls the attached microphone for up to `wait` without truncating mid-utterance,
    /// returning `Ok(Some(candidates))` when a full command finishes.
    pub fn poll_recognize_command(
        &mut self,
        phrases: &[String],
        wait: std::time::Duration,
    ) -> Result<Option<Vec<SpeechCandidate>>, SpeechError> {
        if let Some(pcm) = self.pending_pcm.pop_front() {
            return self.transcribe_pcm(&pcm, phrases).map(Some);
        }
        let mic = self.mic.as_mut().ok_or_else(|| {
            SpeechError::Unavailable("no microphone attached and no queued PCM audio".into())
        })?;
        match mic.poll_next_utterance(wait)? {
            Some(pcm) => self.transcribe_pcm(&pcm, phrases).map(Some),
            None => Ok(None),
        }
    }
}

impl SpeechEngine for TranscribeCppEngine {
    fn name(&self) -> &'static str {
        "transcribe-cpp"
    }

    fn recognize_command(
        &mut self,
        phrases: &[String],
    ) -> Result<Vec<SpeechCandidate>, SpeechError> {
        if let Some(pcm) = self.pending_pcm.pop_front() {
            return self.transcribe_pcm(&pcm, phrases);
        }

        let mic = self.mic.as_mut().ok_or_else(|| {
            SpeechError::Unavailable("no microphone attached and no queued PCM audio".into())
        })?;
        let pcm = mic.capture_next_utterance(None)?;
        self.transcribe_pcm(&pcm, phrases)
    }
}

fn collect_bias_phrases(phrases: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in phrases {
        if p.contains('<') || p.contains('{') || p.contains('[') || p.contains('(') {
            continue;
        }
        let cleaned = normalize_command_transcript(p);
        if !cleaned.is_empty() && seen.insert(cleaned.clone()) {
            out.push(cleaned);
            if out.len() >= MAX_BIAS_PHRASES {
                break;
            }
        }
    }
    out
}

/// Normalizes an STT transcript for spoken command matching: lowercases, removes outer/stray
/// punctuation, and collapses whitespace.
pub fn normalize_command_transcript(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_alphanumeric() {
            cleaned.push(ch.to_ascii_lowercase());
        } else if ch == '\'' {
            let prev_ok = i > 0 && chars[i - 1].is_ascii_alphanumeric();
            let next_ok = i + 1 < chars.len() && chars[i + 1].is_ascii_alphanumeric();
            if prev_ok && next_ok {
                cleaned.push('\'');
            } else {
                cleaned.push(' ');
            }
        } else {
            cleaned.push(' ');
        }
    }

    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Converts a raw recognizer transcript into ranked command candidates using `active_phrases`.
pub fn rank_transcript_candidates(
    raw_transcript: &str,
    active_phrases: &[String],
) -> Vec<SpeechCandidate> {
    let normalized = normalize_command_transcript(raw_transcript);
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut candidates = vec![SpeechCandidate {
        transcript: normalized.clone(),
        confidence: 0.95,
    }];

    let mut fuzzy: Vec<(f32, String)> = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(normalized.clone());

    for phrase in active_phrases {
        if phrase.contains('<')
            || phrase.contains('{')
            || phrase.contains('[')
            || phrase.contains('(')
        {
            continue;
        }
        let norm_phrase = normalize_command_transcript(phrase);
        if norm_phrase.is_empty() || !seen.insert(norm_phrase.clone()) {
            continue;
        }
        let sim = normalized_similarity(&normalized, &norm_phrase);
        if sim >= 0.78 {
            fuzzy.push((sim, norm_phrase));
        }
    }

    fuzzy.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (sim, phrase) in fuzzy.into_iter().take(5) {
        candidates.push(SpeechCandidate {
            transcript: phrase,
            confidence: 0.75 * sim,
        });
    }

    candidates
}

fn normalized_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let max_len = a_chars.len().max(b_chars.len());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein(&a_chars, &b_chars);
    1.0 - (dist as f32 / max_len as f32)
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];

    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b.len()]
}
