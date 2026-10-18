/**
 * Default model names for transcription engines.
 * IMPORTANT: Keep in sync with Rust constants in src-tauri/src/config.rs
 */

/**
 * Default Whisper model for transcription when no preference is configured.
 * Quantized large-v3-turbo: good Mandarin accuracy at ~550 MB.
 */
export const DEFAULT_WHISPER_MODEL = 'large-v3-turbo-q5_0';

/**
 * Default Parakeet model for transcription when no preference is configured.
 * This is the quantized version optimized for speed.
 */
export const DEFAULT_PARAKEET_MODEL = 'parakeet-tdt-0.6b-v3-int8';

/**
 * SenseVoice-Small (FunASR), served by the same ONNX engine as Parakeet.
 * Chinese-first; keep in sync with SENSE_VOICE_MODEL_NAME in parakeet_engine.rs
 */
export const SENSE_VOICE_MODEL = 'sense-voice-small-int8';

/**
 * Model defaults by provider type
 */
export const MODEL_DEFAULTS = {
  whisper: DEFAULT_WHISPER_MODEL,
  localWhisper: DEFAULT_WHISPER_MODEL,
  parakeet: DEFAULT_PARAKEET_MODEL,
} as const;
